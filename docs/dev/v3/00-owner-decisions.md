# v3 harness: owner decisions (2026-11-05/06, binding)

These are the owner's confirmed decisions, recorded from the discussion. They are the source of truth for every v3 graph node. If a spec or implementation conflicts with this file, this file wins. If something here is ambiguous, ask in your report instead of guessing.

Reference material (read before designing):
- `~/Projects/Personal/droid-harness-decompose/DECOMPOSITION.md`: Factory Droid CLI tools, permissions, settings, hooks, sandbox
- `~/Projects/Personal/droid-harness-decompose/HARNESS-ARCHITECTURE.md`: turn loop, tool-call gauntlet, permissions, context assembly, §9 "rebuild as pi extension", §10 invariants
- `~/Projects/Personal/droid-harness-decompose/HARNESS-SCORE-LOGIC.md`
- `~/Projects/Personal/droid-harness-decompose/HARNESS-COMPACTION-CONTEXT.md`: request assembly, prefix cache, compaction internals
- `~/Projects/Personal/droid-harness-decompose/DROID-PROMPTS.md`: verbatim system prompt, session-start reminders, all tool descriptions and schemas, mid-session reminders, compaction prompts
- `~/Projects/Personal/droid-harness-decompose/artifacts/tool-defs/*.json`: raw Droid tool schemas
- `docs/dev/v3/ref-dsh-superagent.md`: DeepSeek dsh-superagent explained (superagent reference)
- pi codemode docs: `/home/oi/.local/share/mise/installs/node/24.22.1/lib/node_modules/@earendil-works/pi-coding-agent/docs/codemode.md` (and sibling docs)
- Devin memory and dreaming: https://devin.ai/blog/memory-and-dreaming

## A. System prompt
- A1. Follow Droid's system prompt (docs/dev/v3/ref-droid-prompts.md §1) closely in structure and rules, retuned to Oino: the harness name is "Oino" (never "Droid" or "Factory"), Oino tool names, Oino features. Drop Factory-only content (FetchUrl → Factory docs, missions, automations, Co-authored-by factory-droid).
- A2. The system prompt is FROZEN for the whole session (see C).

## B. Tools: keep OUR lowercase names, adopt Droid's shapes and behaviour
| Ours (name kept) | Becomes |
|---|---|
| `read` | Droid Read: `file_path` (absolute), `offset`, `limit` (default 2400), `line_numbers` (`12→text`), `image_quality` (default/high); a bare read of a long file opens as a numbered outline of declaration lines; short files return whole; images ≤5 MB; PDFs ≤3 MB |
| `ls` | Droid LS: `directory_path`, `ignorePatterns` |
| `find` → **`glob`** | Droid Glob: `patterns[]`, `excludePatterns`, `folder`. Name `glob`; keep a `find` alias for replaying old sessions |
| `grep` | Droid Grep shape (`pattern`, `path`, `glob_pattern`, `output_mode` (content/files_with_matches/count), `case_insensitive`, `type`, `context_before/after`, `context`, `line_numbers`, `head_limit`, `multiline`, `fixed_string`). Backend = in-process ripgrep crates by DEFAULT. Settings toggle "Semantic search (zvec-grep)" switches to zvec-grep (see I) |
| `write` | Name kept; Droid Create behaviour: overwrite or create, refuse a directory path (`Cannot create file at "X" because a directory already exists there.`), atomic temp+rename write, keep line endings/encoding/BOM, ensure trailing newline, result `{success, file_path}` + new-diagnostics reminder |
| `edit` | Best of both: KEEP our batched `edits[]` (`oldText`/`newText`), ADD per-edit `change_all`, ADD Droid's stale-read guard (`File "X" has been modified externally since it was last read. Please use the Read tool…`; an untouched file is auto-tracked, not refused), atomic write that keeps encoding/line endings/BOM, a diff summary in the result, and a reminder listing only NEW diagnostics |
| *(new)* `apply_patch` | Codex-style `*** Begin Patch` freeform patch (`{input}`), create/update/delete/move. Shown ONLY when the model id matches `*gpt*` (also codex/o-series per Droid's inference); when shown it is offered alongside write/edit (the owner wants all three implemented) |
| `bash` / `powershell` | Name kept; Droid Execute shape: `command`*, `summary`*, `riskLevel`* (low/medium/high, Droid's rubric verbatim in spirit), `timeout` (default **60 s**), `fireAndForget` (returns a task id + log path; result delivered automatically on exit; no polling). 32 KiB head+tail output budget, rest to a log file. New shell per call |
| `bg_run/bg_status/bg_logs/bg_kill` | Replaced by full Droid parity: `fireAndForget` on bash plus `task_output(task_id, block, timeout)` and `task_stop(task_id)`. ONE id space shared with subagent tasks (run_subagent background ids). Keep the work-tray UI showing every arg and live status. Keep the mid-run bash detach as an extra |
| watchdog | Its Decision-model judging is REMOVED entirely (the default timeout + fireAndForget replace it). Session caps may stay |

Old sessions must still replay (alias old names/args where needed).

## C. Prefix cache: append-only context
- C1. Slot 1 = system prompt (frozen). Slot 2 = ONE synthetic user message before the first real user message holding `<system-reminder>` blocks: tool catalog + deferred-tool list, subagents, system info + cwd, date, start-of-session workspace snapshot (ls + git status), AGENTS.md/context files. Written once, never edited.
- C2. Everything that changes later (tools added/removed, model change, MCP connecting, mode changes, extension state, skills, todo nags, diagnostics, media limit) is APPENDED at the tail as `<system-reminder>` content. Never rewrite earlier messages or the system prompt mid-session. Stale reminders may be pruned only when the pruning keeps the cached prefix byte-identical (Droid reuses ids).
- C3. Today's breakers to remove: `collapse_system_messages` folding section/tool patches into the leading system message when `supportsMidConvoSystemMessages` is false; extensions mutating `systemPromptOptions` per turn (skill-registry, long-horizon, subagents, fusion, browser); system-prompt section diffs.
- C4. The system prompt tells the model that `<system-reminder>` blocks are trusted harness context, not user text.

## D. Skills
- D1. Skills are NOT hardcoded or listed in the system prompt.
- D2. They are appended (C2) when the decision engine picks them for the turn, or by a tool when relevant (e.g. a tool result carries a skill hint). Track which skills are already announced so they aren't repeated; carry the announced/loaded set across compaction.
- D3. The Decision model (jev) stays, for SKILLS ONLY in this round. Its workflow and watchdog uses go away (replaced by B/E). Long-horizon mode routing and session renaming stay untouched for now ("later").

## E. Autonomy (shell risk): copy Droid
- E1. The model declares `riskLevel` on every bash call; the harness enforces a ceiling. A chained command takes the max risk; redirecting to a file is ≥ medium; user authorization does not lower risk (Droid rubric).
- E2. 3 levels: Low / Medium / High. Composer chip "Autonomy ▮▮▯" like Droid's; a click cycles the levels; hover tooltip ("Autonomy: High (allow all commands)"). Read-only remains Plan mode.
- E3. Above the ceiling: interactive chat shows an approval card (allow once / always / deny); unattended runs (cron, superagent workers, background subagents) block with an actionable message that names the fix ("needs Autonomy High").
- E4. Declarative permission rules engine (versioned rules, scopes, deny → legacy deny → ask → allow → default) with actionable block reasons. Replaces the `workflow` ask/auto/full modes and its jev judging. Superagent tickets carry their own autonomy level (per ticket).

## F. Compaction: Droid's architecture
- F1. Anchored, append-only `compaction_state` record in the session stream (summary text/tokens/kind, anchor message id+index, removedCount, retained skills/images, loaded deferred tool names); every message tagged with its compaction epoch; reload = splice at the anchor; the UI keeps the full history visible (render cutoff).
- F2. Iterative DELTA summarization: previous summary + messages since the anchor; soft length cap; strip the old TODO block before feeding back; append the current todos.
- F3. Triggers: post-turn threshold from REAL provider usage (limit resolution: session override → per-model → global → model default → 250k, clamped to max input); in-turn context-overflow compaction that RETRIES the same turn with requireProgress; manual `/compact` with custom instructions. Suppression ref so compaction never loops when usage won't drop. A failed threshold compaction is non-fatal. PreCompact hook can block manual only.
- F4. Summarizer window fitting: instructions ≤25% of the window, 4k reserve, pinned tail + `[N earlier messages omitted…]`, shrink ladder 0.9 → 0.75 → 0.5, resample/escalate on empty output.
- F5. LLM compaction prompt = Droid's VERBATIM (docs/dev/v3/ref-droid-prompts.md §5; the injected summary message names Oino instead of Droid). After compaction: reset the prompt-cache snapshot, re-announce retained skills/deferred tools.
- F5a (owner, 2026-11-07). Use only the adaptive body (ref §5.1) plus §5.3/§5.4/§5.5. Drop the classic 10-section body (§5.2); no setting to pick it.
- F6. KEEP the existing vcc / llm compaction toggle so the user chooses which compactor runs; both plug into the same F1/F3 architecture.

## G. Loop guards (normal loop, every session, not just missions)
- G1. TodoWrite suggestion after 5 tool calls with no list; stale-list reminder after 20 calls with no update; settle nag (once) when the model ends the turn with open todos ("You may be ending the turn, but the TodoWrite plan still has open items…"); continue the loop after the nag.
- G2. Stop after 3 consecutive empty responses (no text, no tools) with a user-visible notice; stop on repeated identical tool calls; resample degenerate/looping output up to 3 times; "Continue." nudge after a length-truncated empty reply.
- G1a (owner, 2026-11-07). Stale-list reminder text: only the prefix "IMPORTANT: Your todo list has pending items but hasn't been updated" exists in the Droid binary (it is a detection substring; the full text is not recoverable from ~/Projects/Personal/droid-harness-decompose). Use that prefix verbatim and finish it as: "IMPORTANT: Your todo list has pending items but hasn't been updated in a while. Update it with todowrite to reflect your current progress: mark finished items completed and keep exactly one item in_progress."
- G3. Steering note when a follow-up user message arrives mid-run (Droid §4.4).

## H. Codemode, deferred tools, ToolSearch
- H1. Bring codemode back (pi parity: see the pi codemode docs).
- H2. Add `tool_search` (Droid ToolSearch: `select:<names>` + keyword), the "Deferred tools:" list in slot 2, a catalog-update reminder when tools appear mid-session, and loaded deferred tools retained across compaction.

## I. zvec-grep (semantic search)
- I1. Settings toggle (default OFF = ripgrep). When ON, `grep` can run zvec-grep (zg, github.com/zvec-ai/zvec-grep, Apache-2.0): exact mode (in-process ripgrep crates, ~34 MB, 30 ms) needs no index; semantic/hybrid needs a per-project index.
- I2. Index stays at `<project>/.zvec-grep/` (owner accepted). Never build an index silently: building needs explicit user consent. Model default: a small local model (e.g. `local/potion-code-16m-v2`, 31 MB).
- I3. No separate zg daemon: run zg in direct mode, or embed the zg engine library in unipi-oino-host (always running in tray/app). Measured: index build peak ~650 MB, cold query ~0.8 s / ~400 MB. Unload the model when idle. Ship the CPU build only (no CUDA).

## J. Devin memory and dreaming parity (keep mempalace)
- J1. Match Devin's Memory and Dreaming concepts; the backend STAYS mempalace (no MEMORY.md, no git memory drive).
- J2. Dreaming: a daily background pass at a user-chosen time; consolidates overlapping notes, removes transient details, finds lessons missed during sessions, removes stale memories no session used (needs per-memory usage tracking), keeps source session links and explicit preferences. Keep our skill/check proposals for approval.
- J3. Dream settings must be visible and usable: a model picker (not free text), schedule (daily time / frequency), enabled, inspect recent dream runs. Today they hide behind an unclickable-looking "Settings" text in Settings › Memory › Dream.

## K. Bugs to fix in this round
- K1. Settings › Subagents › Edit agent sheet: picking a value in the Thinking dropdown (any Kobalte Select inside a SheetFrame/BoardSheet) closes the sheet before Save. Cause: the select's portaled listbox counts as an outside click. Fix it for all selects inside sheets.

## L. Superagent (owner-approved router and leads)
- L1. Superagent is a workspace-free router chat. It identifies the right project, asking if ambiguous, writes tickets to that project's kanboard (the work record), picks a preset and delegates to its lead. No worktrees.
- L2. At most one unattended normal lead chat per project, in the project directory and visible in the sidebar. The lead can hold several dispatcher-owned ticket claims and works them using its long-horizon mode; it finishes each to in_review with a summary or blocked with a question.
- L3. New work for a live lead is relayed to it: dependent work queues, complementary work steers, urgent work is marked priority. Relay comments are author-tagged. Ask before a mode switch as a soft gate, but allow the switch without asking.
- L4. The host has a master enabled switch and per-project autoStart (default off). Auto-start feeds ready todo tickets to that project's lead. Manual starts always work. A global lead cap is 1–3, default 1; lowering it never kills leads.
- L5. Host-persisted presets configure model, thinking, autonomy, mode, parallelism, review and allowed profiles. Defaults are quick-fix, bug-batch, feature, urgent and research. Lead and Superagent-created ticket autonomy defaults High; preset autonomy wins.
- L6. Needs you contains pending approvals and blocked questions only; in_review is a separate review list. Status includes lead progress and projects. Pending approvals park and resume safely.
- L7. Leads finish when their run and queue end with tickets terminal; remaining in_progress tickets become blocked with the last message. Restart blocks interrupted dispatcher claims for manual retry. Old worker transcripts remain readable.
- L8. The router gets normal memory recall and save behavior, including all-project search and memory tools, but no filesystem, shell or other extensions.

## Accepted reviewer decisions (2026-11-07, owner may override)
- E "always" approval = project-scoped allow rule for the exact normalized command; applies to interactive sessions of that project only; never raises the ceiling; unattended runs ignore it. No session scope.
- E default level = Medium for the global interactive default and legacy tickets without a level; Superagent-created tickets and dispatcher leads default High (L5).

## Out of scope for this graph
- Windows install defects found on coffee (separate graph).
- pi 1.5.0 parity bump (separate task).
- Cutting extra commands/skills beyond what the above replaces.
