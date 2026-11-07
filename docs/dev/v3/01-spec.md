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

**Legacy execution normalization:** implement the layer in new `crates/oino-tools/src/legacy.rs`, owned by wp-tools-fs; wp-exec-bg adds background aliases after it. Preserve original stored arguments and card renderers (`extensions/unipi/background-tasks/src/lib.rs:919`). Normalize before the permission gate, retaining task-id prefixes:

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

**Target:** slot 1 is A's immutable system bytes. Slot 2 is one persisted synthetic user message, before the first real user message, containing ordered `<system-reminder>` blocks: unified catalog and deferred names, subagents, system/cwd, date, initial ls/git status and context files. Capture once; later changes append LLM-only user-role reminders after existing history. Never mutate either slot; compaction changes only the model's tail view, preserving both slots byte-for-byte. The provider prefix comprises the ordered tool declaration array, slot 1, slot 2, then history. At session start freeze declarations in built-in registry order followed by extension names sorted lexically. All MCP tools are deferred at session start; built-ins and non-MCP extension tools start declared. Later tools are deferred and announced in a tail catalog update; `tool_search` appends loaded schemas only. A removed tool remains declared until compaction and returns `tool X is unavailable`. Check apply_patch gating at start and model switch; append a model-change reminder and append its schema if newly eligible. Slot 2 contains no model id. Existing declaration bytes and both slots remain identical; additions are suffixes, not rewrites. On Anthropic, appending a schema causes one full cache miss because tools precede the system prompt; accept this rare, model-initiated cost. `defer_loading` is a later optimization. Adapt providers that cannot accept later system messages by serializing later updates as user-role reminders rather than merging them into slot 1. At compaction rebuild declarations in the same order, dropping removed tools and retaining loaded available tools; reset cache tracking without altering slot bytes. Do not prune stale reminders in v3. Escape literal `<system-reminder>` and `</system-reminder>` tags in all untrusted user text and tool output at provider serialization; only host-created reminder envelopes retain the trusted tags. Preserve stored source bytes.

**Extension API contract:** replace hook return mutation with host-owned append. A request is `{ "type":"append_reminder", "key":"<extension-id>/<topic>", "text":"<plain text>" }`; response `{ "id":"<message-id>", "appended":true|false }`. Host validates ownership of key, wraps text once in `<system-reminder>`, persists an LLM-only entry before next model request, serializes appends in event order, without replacing earlier messages. This is an append operation, not an upsert: never replace existing message bytes. Extensions track their own announcement state in the session stream. Failure to append must be surfaced to the originating extension. Tool results may carry a reminder without altering earlier messages. WIT/host bindings and both native and WASM extension paths must implement the same semantics.

**Migration:** for old transcripts, do not insert slot 2 into old history: append its content as the first tail reminder. Legacy system-role entries and `CompactionEntry.system_message` remain visible in the UI but are excluded from provider requests. Append the v3 system snapshot once. Old reveal entries count as already announced; all other skills are unannounced. `systemPromptOptions` mutations by migrated bundled extensions are removed, not silently applied to slot 1. Keep legacy event deserialization.

**Settings/tests:** no new setting. Test first-turn order, append ordering across reload, model changes, dynamic tools, MCP, and both provider serializers; ensure no sensitive user-authored text is promoted to trusted reminder content.

## D. Skills

**Current:** `crates/oino-resources/src/system_prompt.rs:316` inserts skill sections; registry hook returns mutated options at `extensions/unipi/skill-registry/src/lib.rs:446` and :567. Its judge uses jev (`extensions/unipi/skill-registry/src/judge.rs:207`).

**Target:** never include skills in A's system prompt or C's initial slot. The existing skill Decision model selects relevant skills per turn; emit selected skill metadata/instructions as C reminders or a tool-result hint. Persist announced and loaded names in session state, dedupe repeat announcements, retain them across compaction and reload. Restore selected skills after compaction as required by F without re-running the decision engine for already retained skills. Keep long-horizon routing and session renaming unchanged. Remove Decision-model uses only from workflow and watchdog, not the shared jev transport or skill judge.

**Migration/settings/tests:** interpret existing registry reveal entries as prior announcements and preserve current skill settings. Test no skill text in the immutable prompt, per-turn selection, no duplicates, tool hint, reload and compaction retention; verify workflow/watchdog no longer invoke jev.

## E. Autonomy and permission rules

**Current:** `extensions/unipi/workflow/src/decide.rs:121` configures jev confidence and :138 sends ambiguous commands to jev. `extensions/unipi/workflow/src/bash.rs:1` classifies shell commands. The composer currently has `apps/unipi/src/components/composer/PermissionControl.tsx`.

**Risk enforcement:** Low < Medium < High. Effective risk is max(model declaration, harness floor); the existing `extensions/unipi/workflow/src/bash.rs` classifier may only raise it. Parse bash chains `&&`, `||`, `;`, pipes, newlines, command substitutions `$()`/backticks, process substitution `<()`, nested `bash|sh|zsh -c`, `eval`, `xargs`, `find -exec/-delete`, heredocs and redirects; unknown syntax or unknown commands are High. PowerShell must cover `;`, `|`, `&`, `iex`/`Invoke-Expression`, `-EncodedCommand`, `Start-Process`, `Out-File`, `Set-Content`, `>`, nested `pwsh`/`powershell -Command`, `cmd /c`, `Invoke-Command`, dot-source and `&{}` scriptblocks; unknown syntax or unknown commands are High. File redirects are at least Medium, except `/dev/null`, `$null` and `NUL` as appropriate. Chains take maximum risk, and user authorization never changes classification. Builds, tests and commits are Medium; `push`, `rm -rf` and `sudo` are High. Reject absent or malformed risk except legacy normalization (B). Rules and risk apply after normalization and to aliases, codemode and background execution. Non-shell tools, including side-effecting MCP tools, use rules only, no risk ceiling even at Low; Plan restrictions still apply. Child subagents use min(parent, child) ceiling. Cron, superagent workers and background subagents are always unattended. Worker ceilings use ticket autonomy (child ceilings remain bounded by the parent); cron uses the global default. Foreground child-subagent approvals appear in the parent approval UI. TUI/RPC surfaces unable to display cards are unattended, never hang; ACP maps to `request_permission`.

**Accepted reviewer decisions, recorded in `00-owner-decisions.md` (2026-11-07), owner may override:** global interactive default, legacy-ticket default and new-ticket default are Medium. An interactive “always” writes an exact normalized command allow scoped to that project. It permits that command above the ceiling only in interactive sessions of that project; it never raises the ceiling and is ignored unattended. Explicit deny, legacy deny and ask retain precedence. Other allows do not bypass the ceiling. Plan remains an independent read-only mode.

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

Each subsequent `SessionEntry` carries `compactionSummaryId` equal to the active record id; do not change provider message bytes. `anchorMessage` is the first kept message, inclusive. Anchor at a turn boundary, never between an assistant tool call and its results. Construct the injected summary deterministically from the state so reload is byte-identical. v3 writes only `compaction_state`, not legacy `CompactionEntry`, but reads the latter. A committed state and its anchor must refer to existing messages on the same branch. Reload selects the newest valid state on that branch, forms the model view as immutable slots 1–2, injected summary, then messages from the anchor inclusive; preserve the entire raw transcript for human scrolling. If the anchor is missing, reject the state and surface a recoverable error rather than discard history. Read legacy compaction entries as legacy records; do not rewrite them. Never append a partial compaction state on failure.

Per owner F5a, the LLM summarizer uses only `ref-droid-prompts.md` adaptive §5.1 plus §5.3, §5.4 and §5.5 verbatim, with Droid → Oino only and conditional interpolation; omit the unrecovered `{ur}` identity prefix. Drop classic §5.2 with no setting to select it. At a 6000-token previous-summary soft cap use the §5.4 IMPORTANT length instruction. Summarize the previous summary plus messages from its inclusive anchor, stripping the previous TODO block and appending current todos. Keep vcc/llm selection, both using the same persistence, trigger and recovery machinery; vcc writes `vcc_summary` without the LLM delta step. Clamp custom instructions to 25% of summarizer context; reserve 4k tokens, pin the recent tail and insert the omitted-message marker. Retry context failures on the 0.9 → 0.75 → 0.5 shrink ladder; resample empty/degenerate responses then escalate once to the session main model; final failure preserves history, and threshold failure is nonfatal. Preserve original data if every attempt fails.

Post-turn threshold uses actual provider input usage including cache reads; resolution is session override → model/family → global → model default → 250k, clamped to max input. Suppress repeated threshold compaction for the same session/model/limit until usage falls below the limit; failed threshold compaction is nonfatal. On in-turn overflow compact with `requireProgress` and retry only the failed model request, never completed tools; if no messages are removed, return the original failure. `/compact` accepts custom instructions; PreCompact may block only manual compaction. Reset the cache snapshot after success, then re-announce retained skills and loaded deferred tools without altering slots 1–2.

**Settings:** preserve vcc/llm toggle, existing compactor model and `compaction.enabled`; add only `compaction.tokenLimit`, `compaction.tokenLimitPerModel` and a session override. Existing `keep_recent_tokens` pins the tail for both compactors; replace `reserve_tokens` with the limit chain. `context_edit` may target only tail entries the provider has not seen; reject extension drafts targeting earlier entries (`crates/oino-core/src/session/boundary.rs:66`). **Tests:** legacy and epoch reload, branch anchor validation, crash before commit, no data loss, retained images/skills, iterative delta/TODO replacement, all trigger and failure paths, provider usage accounting, cache-prefix byte identity.

## G. Loop guards

**Current:** `crates/oino-agent/src/agent_loop.rs:128` polls steering and :213 executes tool batches; it has no v3 todo/empty-response guard at those points.

**Target:** every normal session counts completed tool calls since a list was created or updated. Suggest TodoWrite after five calls without a list; remind after 20 calls without an update; once per ending attempt, if open items remain, append the verbatim reminders from `ref-droid-prompts.md` §§4.1, 4.2 and 4.3 at their respective triggers (for §4.2 use owner G1a’s complete text below); append the settle nag and continue. A new follow-up user message during execution gets the §4.4 steering note, not cancellation. Stop after three consecutive responses with neither text nor tool calls, and surface a notice. Stop on three consecutive identical calls, defined by tool name plus canonical JSON arguments. Resample degenerate/looping output at most three times, only before any tool in that response runs. A length-truncated empty response gets `Continue.` before retry. Reset counters at appropriate turn/list progress boundaries and ensure resampling never executes the same side effect twice.

Owner G1a supplies the complete stale-todo reminder (replacing the reference’s partial §4.2):

```text
IMPORTANT: Your todo list has pending items but hasn't been updated in a while. Update it with todowrite to reflect your current progress: mark finished items completed and keep exactly one item in_progress.
```

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

The Superagent is a workspace-free router chat: resolve the user's project (ask if ambiguous), create and organize its kanboard tickets, select a preset and hand work to its project lead. The board is the record of work; no worktrees. Router tools include cross-project kanboard, project resolver, start/stop/relay/set_mode, ask_user and the memory extension. There are no filesystem or shell tools, or unrelated extensions. Memory search defaults to all projects; normal recall and save hooks apply.

A machine-wide dispatcher holds an advisory lock. It permits one unattended normal lead session per project and a configurable global running-lead cap of 1–3 (default 1). Manual starts work regardless of watcher settings. The watcher requires both the global enabled switch and the project's autoStart toggle (default off), and batches ready todo tickets. Lowering the cap never kills sessions. A lead is listed in the sidebar and gets a brief containing each ticket's id, title and body excerpt, with instructions to claim and work them in its mode and finish each to in_review with a summary or blocked with a question. Dispatcher-owned claims are idempotent for the same session and exempt from the stale-claim reaper.

For a live lead, new work is delivered to the same session: dependent work queues as follow-up, complementary work steers at the next step, and priority prepends an urgent scheduling instruction. Persist author-tagged ticket comments. A mode switch uses the long-horizon command runner, taking effect before the next turn; ask the user first as a soft gate. Presets are host-persisted and set model, thinking, autonomy and mode; parallelism, review and profiles describe execution. The shipped quick-fix, bug-batch, feature, urgent and research presets and default preset id are defined in `superagent-protocol.md`. Superagent-created tickets and leads default to High autonomy; the preset wins.

Status reports leads, projects, pending approvals and blocked questions in needsYou, and in_review separately in review. A lead finishes when its run and queue end and tickets are terminal; in_progress claims are blocked using the last message. On restart each interrupted dispatcher claim is blocked with `Worker was interrupted; move to todo to retry`, never requeued automatically. Approvals park and resume the claiming lead. Old worker sessions remain replayable.

**Host protocol:** authenticated request and reply shapes, errors, status, start, stop, relay, set_mode, configuration, presets and approval events are specified in `superagent-protocol.md` and `worker-approvals-protocol.md`.

**UI:** center router chat, project toggles and lead status, with Needs you separate from review. No user-steer chip for host/agent-authored messages. Test multi-ticket dispatch and feed, cap, modes, approvals, lifecycle, memory isolation and crash recovery.

## Already implemented

- J dream backend/UI: `80d2e16e`, `8e6dbda2`, `de6d39c6`, `91e8ce60`; R15's remaining usage-history/metadata gap is assigned above.
- K sheet select: `8abc78b3`; no new package.

## Work packages

The table assigns exactly one owner per path. Directory ownership excludes explicitly assigned child paths. Paths labelled **new** and all acceptance targets not yet present are implementation targets, not existing artifacts. Other named repository paths exist. Cross-package edits are allowed only from a direct or transitive dependent, after the owner finishes. Tests and fixtures are owned by the package in their acceptance row unless explicitly assigned otherwise; wp-autonomy-ui also owns `apps/unipi/src/components/phone/SheetSelect.test.tsx`. No implied dependency or global serialization order exists.

| Package | Owned paths and responsibilities | Depends on |
|---|---|---|
| wp-cache | `crates/oino-session/src/entry.rs`, `crates/oino-ai/src/transcript.rs`, `crates/oino-core/src/session/{prompt,tools,mod,hooks,host}.rs`, `crates/oino-resources/src/system_prompt.rs`, `extensions/sdk/oino-provider-kit/src/{anthropic,completions,codex,google,azure,bedrock,responses}.rs`; prefix and legacy snapshots/replay | none |
| wp-tools-fs | `crates/oino-tools/src/{read,write,find,grep,ls,info,lib,registry}.rs`, `crates/oino-tools/src/edit/`, **new** `crates/oino-tools/src/{apply_patch,legacy}.rs`; file tools, registration and legacy normalization | none |
| wp-ext-reminders | `extensions/unipi/{skill-registry,long-horizon,subagents,fusion,browser,mcp,memory,dream}/`, `wit/oino-extension/*.wit`, `crates/oino-extension-api/`, `crates/oino-wasm-host/`; reminder bindings and R15 gap only | wp-cache |
| wp-loop-guards | `crates/oino-agent/src/agent_loop.rs`; guards | wp-cache |
| wp-exec-bg | `crates/oino-tools/src/bash/`, `extensions/unipi/{background-tasks,watchdog}/`; task output/stop and shared id registry | wp-tools-fs, wp-ext-reminders |
| wp-codemode | `crates/oino-tools/src/js.rs`; codemode, tool_search and deferred list | wp-tools-fs, wp-ext-reminders, wp-loop-guards, wp-exec-bg |
| wp-autonomy | `extensions/unipi/workflow/`, **new** `crates/oino-core/src/session/permission.rs`, `crates/oino-settings/`, `crates/oino-rpc/`, `crates/oino-acp/`; host gate, settings and approval wire | wp-exec-bg, wp-codemode |
| wp-autonomy-ui | `apps/unipi/src/components/composer/PermissionControl.tsx`, **new** `apps/unipi/src/components/composer/ApprovalCard.tsx`, **new** `apps/unipi/src/components/chat/RenderCutoff.tsx`, `apps/unipi/src/lib/session/turns.ts`, `apps/unipi/src/screens/settings/`, `apps/unipi/src/screens/Settings.tsx`, `crates/oino-tui/`; approvals, cutoff and Settings integration | wp-autonomy |
| wp-compaction | `crates/oino-core/src/compaction/`, `crates/oino-core/src/session/{compaction_flow,boundary}.rs`, `extensions/unipi/compactor/`; **new** `crates/oino-settings/src/v3_compaction.rs` and `apps/unipi/src/screens/settings/CompactionSettings.tsx`; compaction and its settings | wp-cache, wp-ext-reminders, wp-autonomy-ui |
| wp-zvec | **new** `crates/oino-tools/src/zvec/`, `crates/oino-settings/src/v3_search.rs`, `apps/unipi/src/screens/settings/SearchSettings.tsx`; semantic search and its settings | wp-tools-fs, wp-codemode, wp-autonomy-ui |
| sa-backend | `apps/unipi/crates/unipi-kanboard/`, `apps/unipi/crates/unipi-host/src/{protocol,cron_serve,settings,lib}.rs`, **new** `apps/unipi/crates/unipi-host/src/superagent.rs`, `extensions/unipi/kanboard/`; dispatch, cron global-default enforcement, watcher settings and host protocol | wp-autonomy |
| sa-ui | `apps/unipi/src/components/shell/`, **new** `apps/unipi/src/components/superagent/`; Superagent page, watcher controls and author rendering | sa-backend, wp-autonomy-ui |

**Declared ordered edits** (arrows below mean edit order, not dependency direction):

1. `entry.rs`: wp-cache → wp-compaction for epochs/state.
2. `crates/oino-tools/src/{info,lib,registry}.rs`: wp-tools-fs → wp-exec-bg → wp-codemode → wp-zvec for execution, discovery, then conditional grep/zvec registration. `legacy.rs`: wp-tools-fs → wp-exec-bg for background aliases. `grep.rs`: wp-tools-fs → wp-zvec.
3. `agent_loop.rs`: wp-loop-guards → wp-codemode → wp-autonomy for declaration handling then permission integration.
4. `extensions/unipi/subagents/`: wp-ext-reminders → wp-exec-bg → wp-autonomy for ids then ceiling integration.
5. Settings crate and Settings UI: wp-autonomy and wp-autonomy-ui create the shared settings registration hooks before finishing; the later feature modules use those hooks without requiring edits to shared registration files. Do not add uncompilable imports of modules that do not exist yet. wp-compaction and wp-zvec then implement only their exclusively owned module files above; they do not edit common settings files. Watcher settings stay in sa-backend’s host settings; watcher controls stay in sa-ui’s Superagent page, not shared Settings. Existing dream controls are retained.
6. WIT/API/WASM bindings: wp-ext-reminders → wp-autonomy for any permission bridge additions. No unspecified subsequent editors.
7. `crates/oino-core/src/session/{prompt,tools,mod,hooks,host}.rs`: wp-cache → wp-ext-reminders → wp-codemode → wp-autonomy → wp-compaction for reminder bridge, discovery, gate, then compaction integration. wp-zvec uses tool registration only; sa-ui uses app paths only.
8. `turns.ts`: wp-autonomy-ui owns cutoff integration → sa-ui edits `isSteer`/author handling. No reverse edit.
9. Host cron integration is implemented by sa-backend using wp-autonomy’s public policy API; wp-autonomy never edits host-owned files.
10. Legacy replay file and fixtures: wp-cache owns `crates/oino-core/tests/v3_legacy_replay.rs` and its fixture directory; wp-tools-fs supplies mapping requirements to wp-cache without editing its files (no dependency). wp-exec-bg → subsequent contributions after wp-cache, including fs/background fixtures; wp-compaction contributes compaction fixtures after wp-exec-bg transitively. No co-ownership.
11. `crates/oino-tools/src/bash/` and `crates/oino-tools/src/js.rs`: wp-exec-bg/wp-codemode respectively → wp-autonomy for permission integration. `extensions/unipi/background-tasks/`: wp-exec-bg → wp-autonomy for alias gating. sa-backend handles cron gating in its own host files; wp-autonomy tests the policy through fake callers.

Concurrency: wp-cache and wp-tools-fs may run together; wp-ext-reminders/wp-loop-guards may run alongside wp-tools-fs once prerequisites permit; wp-autonomy-ui and sa-backend may run together; wp-compaction, wp-zvec and sa-ui may run together. Each concurrent group has disjoint edit sets, including tests and settings modules. No package edits another’s file without a dependency path.

## Acceptance tests

The following are implementation targets, not claims that these new tests already exist. Every named test must be implemented and pass; commands run from repository root unless a working directory is stated. All new test files and fixtures named below are explicitly **new** unless already present. Paths for new test files are relative to existing crates or the existing app. For Rust, `--test` selects an integration test target; case filters select named tests. Fixtures must be checked in alongside their tests.

| Package | Named cases and exact commands |
|---|---|
| wp-cache | `crates/oino-core/tests/v3_prefix.rs::prefix_stable_across_{tool_search_load,model_switch,mcp_connect,skill_announce,compaction,reload}` records provider requests for anthropic and completions serializers, compares slot/message bytes and permits declaration suffixes only. `v3_prefix.rs::prompt_excludes_factory_content` checks forbidden names and `::untrusted_reminder_tags_are_escaped` checks user/tool spoofing. Run `cargo test -p oino-core --test v3_prefix`. |
| wp-tools-fs | `crates/oino-tools/tests/v3_fs.rs::{schema_and_paging,atomic_write_and_edit,patch_model_gate,windows_atomic_replace}` covers schemas, read limits, CRLF/BOM, stale edits and patch operations; run `cargo test -p oino-tools --test v3_fs`. `windows_atomic_replace` is `cfg(windows)`. |
| wp-exec-bg | `crates/oino-tools/tests/v3_exec.rs::{background_registry_and_stop,timeout_and_detach,windows_job_tree}` covers shell and subagent ids, output and process tree; run `cargo test -p oino-tools --test v3_exec`. `windows_job_tree` is `cfg(windows)`. |
| wp-ext-reminders | `crates/oino-core/tests/v3_reminders.rs::{append_order_and_reload,skill_mcp_catalog_updates}` covers WIT/native/WASM reminders. **New** `extensions/unipi/memory/src/usage.rs::tests::memory_usage_and_prune` covers search/injection/read use, sidecar import, 30-day boundary and never-used/preferences; run `cargo test --manifest-path extensions/Cargo.toml -p unipi-memory memory_usage_and_prune`. Run `cargo test -p oino-core --test v3_reminders` and `cargo xtask wasm`. |
| wp-loop-guards | `crates/oino-agent/tests/v3_guards.rs::{todo_thresholds,empty_and_identical_calls,resample_before_tools}` covers exact recovered reminders, steering and no repeated side effects; run `cargo test -p oino-agent --test v3_guards`. |
| wp-compaction | `crates/oino-core/tests/v3_compaction.rs::{crash_before_append,anchor_keeps_tool_pair,overflow_retry_does_not_rerun_tools,epoch_reload_and_legacy,vcc_and_llm_triggers}` covers failures and deterministic reload; run `cargo test -p oino-core --test v3_compaction`. |
| wp-codemode | `crates/oino-core/tests/v3_codemode.rs::{search_and_deferred_reload,sandbox_and_permission_gate}` covers exact/keyword lookup, MCP and execution; run `cargo test -p oino-core --test v3_codemode`. |
| wp-autonomy | `crates/oino-core/tests/v3_autonomy.rs::{risk_table,unattended_and_child_ceiling,approval_protocol,rpc_tui_unattended_fail_closed,acp_request_permission,gate_covers_alias_fireandforget_codemode_subagent_cron,windows_powershell_floor}` tests `ls && rm -rf x` declared low → high, `echo > f` → medium, `echo > /dev/null` → low, PowerShell `iex` and `Remove-Item -Recurse`; unattended High returns exactly `needs Autonomy High`. Run `cargo test -p oino-core --test v3_autonomy`; Windows case is `cfg(windows)`. |
| wp-autonomy-ui | `apps/unipi/src/components/composer/AutonomyControl.test.tsx` cases `cycles_levels_and_tooltips`, `approval_allow_once_always_deny`, `render_cutoff_preserves_history`; run `cd apps/unipi && npx vitest run src/components/composer/AutonomyControl.test.tsx && npm run typecheck`. wp-autonomy owns the named TUI/RPC/ACP and gate-path cases above; this package owns only its UI tests. |
| wp-zvec | `crates/oino-tools/tests/v3_zvec.rs::{exact_without_index,consent_and_missing_index,project_isolation_and_idle_unload,windows_msvc_cpu}`; run `cargo test -p oino-tools --test v3_zvec`. Windows case is `cfg(windows)`. |
| sa-backend | `apps/unipi/crates/unipi-host/tests/v3_dispatcher.rs::{watcher_manual_race_one_run,two_todos_one_project_worker,crash_mid_launch_blocks_without_second_session,windows_tray_dispatcher}`; run `cargo test --manifest-path apps/unipi/Cargo.toml -p unipi-host --test v3_dispatcher` and `cargo test --manifest-path apps/unipi/Cargo.toml -p unipi-kanboard --test v3_kanboard`; `apps/unipi/crates/unipi-kanboard/tests/v3_kanboard.rs::{dispatcher_reaper_does_not_requeue,same_session_claim_is_idempotent,ticket_defaults_and_relay}` covers reaper, claims, ticket defaults and relay. Windows case is `cfg(windows)`. |
| sa-ui | `apps/unipi/src/components/superagent/SuperagentPage.test.tsx::{worker_and_needs_you,watcher_and_relay,author_not_steer}`; run `cd apps/unipi && npx vitest run src/components/superagent/SuperagentPage.test.tsx && npm run typecheck`. |

**Legacy replay, owned by wp-cache; ordered contributions as above:** `crates/oino-core/tests/v3_legacy_replay.rs::{old_tool_arguments,legacy_compaction_and_context_edit,workflow_and_reveal,append_only_original_bytes}` uses `crates/oino-core/tests/fixtures/v2/*.jsonl` covering find, bg_run/bg_logs, relative read, old grep arguments, legacy `system_message`, context_edit, workflow-mode and reveal entries. Assert original file bytes unchanged with only new lines appended. Run `cargo test -p oino-core --test v3_legacy_replay` after coordinated integrations.

**Already-implemented K regression:** run `cd apps/unipi && npx vitest run src/components/phone/SheetSelect.test.tsx`; wp-autonomy-ui adds the **new** case `subagents_thinking_select_keeps_sheet_open` if absent. Before close run all `cfg(windows)` cases and dispatcher in the tray on an actual Windows host; non-Windows CI does not satisfy that gate.

## Review resolution

| Item | Resolution |
|---|---|
| R1 | C freezes slot and declaration bytes, defers tools, appends schemas and checks model changes; acceptance records both serializers. |
| R2 | A/C append snapshot and tail reminder, exclude old system content from provider, preserve display and reveal state. |
| R3 | B provides legacy argument mappings, normalized permission gating and card preservation. |
| R4 | E records supervisor-accepted reviewer decisions; owner may override. |
| R5 | E specifies effective floor, shell parsing, all entry points and unattended behavior. |
| R6 | Work packages assign features and shared files; declared edits coordinate non-owners. |
| R7 | Twelve-package DAG is the requested graph with already-implemented J/K omitted; no total order. |
| R8 | F defines turn-safe anchors, retry, epochs, deterministic persistence and context_edit restriction. |
| R9 | F specifies soft cap, retry and settings; owner F5a selects adaptive §5.1 only, dropping §5.2 without a selector. |
| R10 | L specifies locks, idempotent claim/launch, crash recovery and reaper exclusion. |
| R11 | L defines workspace-free session, tools and historical command rendering. |
| R12 | B/E/I/L specify Windows behavior and acceptance requires Windows host. |
| R13 | E specifies chip, approval and cutoff UI. |
| R14 | I specifies in-process engine, mode snapshot, refusal and idle unload. |
| R15 | J records existing sidecar behavior and assigns remaining metadata/prune gap to wp-ext-reminders. |
| R16 | G specifies guard counts and verbatim reference reminders, with the complete owner G1a stale-todo text replacing partial §4.2. |
| R17 | C escapes untrusted literal reminder tags without modifying stored text. |
| R18 | Acceptance table defines new file::case targets, fixtures, exact commands and Windows gate. |

Optional suggestions: stale reminder pruning and the `dedupe` field are omitted (C); `.zvec-grep/.gitignore` contains `*` (I); forbidden system-prompt strings are tested (acceptance); model download requires index-build consent (I).

### Re-review resolution

Blocking fixes 1–5: turns ownership is corrected; the supervisor’s exact added-edge DAG is applied; host bindings, session integration, registration, TUI/ACP, app surfaces and replay have explicit owners; unattended classes/foreground routing are explicit; named autonomy tests cover every path. All brief non-blocking clarifications are incorporated above, including the memory test location and OS advisory lock. Owner answers in `00-owner-decisions.md` commit `3a7ddc11` close both prior questions and record the accepted autonomy decisions.

### Open owner questions

None.
