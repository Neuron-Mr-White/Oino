# Droid harness — context lifecycle & compaction (the missing chapter)

Companion to `DECOMPOSITION.md` (tools, prompts, permissions) and `HARNESS-ARCHITECTURE.md`
(turn loop, tool-call gauntlet). This chapter covers what was previously only inferred:
how a request is assembled, how the prompt cache is protected, how token usage is accounted,
and the **full compaction subsystem** (trigger, summarizer, persistence, post-compaction
re-annotation).

Method: same carved Bun modules as before (`artifacts/bun-modules/r*.txt`), identified by
log-string anchors (`[Compaction] ...`, `[Session] ...`) and cross-checked across chunks.
Recovered source paths: `src/hooks/compaction/{CompactionManager,Summarizer,summaryInput,
summaryMeta,retention,documentTokens,transcriptBudget,enums}.ts`,
`src/prompts/_utils/generateIterativeSummarizationSystemPrompt.ts`, `src/commands/compact.ts`,
`src/utils/messages/persistSystem.ts`. Evidence cited as `r####` region + anchor string.

---

## 1. Request assembly (every turn)

Pipeline (minified `En()` in r1460@13784):

```
rawHistory
  → Y2()     filter to LLM-relevant messages (visibility llm_only/user_only split exists throughout)
  → yy()     splice compaction: [summaryMessage, ...messagesAfterAnchor]   (r1269@47506)
  → Ri()     drop stale reminders (old todo nudges, old tool-catalog notices — Mi() matcher r1460)
  → Si()     reconcile fresh reminders: dedupe, replace, and REUSE the same message ids
             so the provider prompt-cache prefix stays byte-identical            (r1460 `Si`)
  → Ii()     walk to the first synthetic user message (id prefix `system-context-…`),
             tag the last stable text block with a cache-pin content id (K9)
  → vi()     prepend the environment header (cwd, model, date, git snapshot) to first user msg
```

Two ideas worth stealing live here:

1. **Reminder reconciliation with id stability** (Si/Ii). Reminders are *mutated in place*
   (same message id, same leading text) instead of appended, so Anthropic-style prompt
   caching survives a reminder refresh. When new messages must precede cached ones, the
   cache-pin marker is moved to the newest stable block.
2. **`prepareMessagesWithCaching(sessionId)`** is called before every send, and
   `resetPromptCacheSnapshot(sessionId)` after every successful compaction — the cache is
   treated as a managed resource with explicit invalidation events.

## 2. Token accounting (what "usage" means)

After each model response the harness computes
`_t = XA({inputTokens: bo+Cn, outputTokens, cacheReadTokens: Sn})` (r1460, just before
`[Compaction] Threshold check`). I.e. the compaction check uses **real usage from the last
response** (input incl. cache reads), not a local estimate. This is why the threshold check
is exact and why the suppression logic (below) can reason about "usage did not go down".

## 3. When compaction fires — three triggers

| Trigger | Where | Mechanics |
|---|---|---|
| **threshold** (post-turn, automatic) | r1460 `[Compaction] Threshold check` | After each completed turn, if `getCompactionThresholdCheckEnabled()` (default on; Alt+X toggles in TUI): `tokens > limit` → compact. `limit = getCompactionTokenLimitForModel(modelId)` resolved as: session override → `compactionTokenLimitPerModel[model]` (also matched by model *family*) → global `compactionTokenLimit` → model registry `defaultCompactionLimit` → constant `Lz = 250000`; always clamped to `min(·, maxInputTokens)` (r1013@~30k). |
| **context_limit** (in-turn, error-driven) | r1460 `ps("context_limit", …)` | When the request fails with a context/signature error: compact **and retry the same turn**. `requireProgress: true` — compaction must remove messages or the original error is rethrown. System-prompt inputs and model may have changed mid-compaction; both are detected and the request is rebuilt. The compaction itself may fall back to a "Droid Core" model (`contextLimitFallbackModelId`, `$a(model)`). |
| **manual** — `/compact` (alias `/compress`) and `/handoff` | `src/commands/compact.ts`; UI `bT()` confirmation dialog (r1399@737800) | User-provided custom instructions are supported (`compaction.instructions` i18n); `/handoff` mode creates a *new* session carrying the summary ("come back later" note). |

**Suppression (anti-loop):** after a threshold compaction, `thresholdCompactionSuppressionRef
= {sessionId, modelId, limit}`. If the next check is still over the *same* limit for the same
model, compaction is skipped with
`"Skipping threshold compaction after previous threshold compaction did not lower usage below
limit"` (r1460). Cleared as soon as one check lands under the limit. A failed threshold
compaction **never fails the turn**: `"Threshold compaction failed; continuing without
compaction"`.

**PreCompact hook** (r1269 `Co()`): fired with `{session_id, transcript_path, cwd,
permission_mode, trigger, custom_instructions, message_count, estimated_tokens, message_id}`.
Exit code 2/3 from a hook **blocks a manual compaction**; for automatic compactions the block
is logged but ignored ("PreCompact hook cannot block automatic compaction").

## 4. The Summarizer (r1460 `Hn`, `nr()`, `DSH()`; beautified copy in /tmp/droid-comp-summarizer.pretty.js)

**Model:** configurable `compactionModel`; default **`gpt-5.4-mini`** (r1162 constant `na`);
selection modes `{Llm: gpt-5.4-mini, Fallback: "fallback", SingleCandidate: "single-candidate"}`.

**Two prompt shapes** (`nr({previousSummaryExists, useAdaptivePrompt})`):
- *Adaptive* (new-style): "Organize around what matters for continuation, not a chronological
  account… Favor a thorough, complete summary… Do not omit useful context merely to keep the
  summary short." Explicit anti-drift rule: *"Do not turn proposals into commitments, reports
  into verified results, partial checks into verified correctness."* Always preserve user
  intent/decisions/corrections/unanswered questions, distinguished from assistant assumptions.
- *Classic* (10 fixed sections): chronological play-by-play → primary request → approach →
  key technical work → questions/clarifications → files & code sections (with full paths +
  repo info) → error resolution → pending tasks → current work → next steps.

**Iterative delta summarization:** if a previous `llm_summary` exists, only messages **after
the previous anchor** are sent, together with `Previous summary:
…​```…​``` These messages were sent since the previous summary: …`, and the model integrates.
A **soft cap** nudges length down when the previous summary exceeds it ("keep your new
<summary> around a similar length by trimming or condensing the least important information").
Output is re-parsed from `<summary>` tags (falls back to "everything after the opening tag").

**Fitting the summarizer's own window** (`or()`, r1460@~370):
- custom instructions clamped to **25%** of the input window (`Qi = 0.25`);
- reserve of **4000 tokens** (`Vi`) on top of previous-summary + instructions;
- remainder is the transcript budget; budget-based selection **pins** recent messages and
  inserts `"[N earlier messages omitted because they did not fit the summarizer's context
  window]"` where older ones were dropped.

**Failure handling — a small state machine** (`Hn` ceilings + retry loop r1460@~640):
- On context-limit refusal: shrink ladder per model — try `maxInputTokens`, then
  `defaultCompactionLimit × 0.9`, then `min(maxInput, 4000)`; when the ladder is exhausted,
  multiply the ceiling by **0.75** (first time) / **0.5** (subsequent) and retry.
- Degenerate/empty output → **resample**; exhausted → **escalate** (different model).
- `stopReason === "length"` → treated as truncated, logged, still usable.
- Moderation-refusal path has its own fallback attempt.
- Final failure surfaces as a user-actionable error: *"Run /compact with a different
  compaction model, or check your model provider"* (`summarizer_empty_response`).

**Summarizer prompt structure** (r1162@8765, structured fallback prompt): priority-ordered
XML sections `turn_context → current_user_message → recent_tool_calls → system_info →
first_user_message → conversation_summary → recent_messages`, each head+tail truncated
(2000/2000 chars default) and HTML-escaped. `turn_context` carries `turn_number, surface,
sub_agent, images_present, has_failed_tool_calls` — the summarizer knows the shape of the
session, not just its text.

## 5. What the summary replaces — and what it keeps

Core routine `qbH()` (r1269@~48700):

1. PreCompact hook gate (manual only, see above).
2. Extract **todos** from the last TodoWrite state (`ko(h)`); latest todos are appended to
   the summary text as `Current TODO List:\n{status} [{priority}] {content}`; the *old* TODO
   block is stripped from the previous summary before it is fed back (`Cr`/`Eo`).
3. Delta slice: messages since the previous `llm_summary` anchor only (non-LLM summary kinds
   are ignored for anchoring: `"Ignoring non-LLM lastSummary for delta compaction"`).
4. New anchor = **id of the last message at compaction time** (`h[M-1].id`).
5. The replacement message is **user-role**, id `injected-summary-{timestamp}`:

   > A previous instance of Droid has summarized the conversation thus far:
   > `<summary> … </summary>`
   > *(+ system-info block)*
   > IMPORTANT: This summary was created by a previous instance of Droid. Files referenced
   > in the summary may not be available until you explicitly view them again.

   (`ye()`, r1269.) A second kind exists: `provider_switch_serialization` — when swapping
   providers mid-session, the prefix is fenced as literal text with the note *"When making
   tool calls, always use your native tool calling schema — do not replicate the text format
   shown in the transcript above."*

6. **Retained lists** survive compaction explicitly: `retainedSkills` and `retainedImages`
   (path + excerpt + imageIndex/imageCount) are stored with the summary record and re-injected
   (`Ke(e)` builds the retention prefix). After compaction, a **skills consolidation** pass
   re-derives the skill roster from the summary (`announcedSkillNames`), and a
   `startPostCompactionDiscoveryEpoch(sessionId, summaryId)` re-opens deferred-tool discovery.

## 6. Persistence & reload

- Every message append carries `meta.compactionSummaryId` = the id of the summary generation
  it belongs to (r1462 Agent loop: `appendMessage(…, {compactionSummaryId:
  this.lastSummaryRef?.id})`). The session file is therefore self-describing across epochs.
- The summary itself is appended to the **same events file** as a `compaction_state` record:
  `{type, id, timestamp, summaryText, summaryTokens, summaryKind, anchorMessage:{id,index},
  removedCount, systemInfo, loadedDeferredToolNames[], systemInfoText, retainedSkills[],
  retainedImages[]}` — written under a `.events.lock` file lock (r1162 `saveCompactionSummary`).
- Reload: scan for the newest `llm_summary`-kind `compaction_state`, splice at
  `anchorMessage.id` (`"[sessionJsonl] Loaded session history from a compaction anchor"`).
- Nothing is deleted: the transcript is append-only; **compaction is a view**, and the UI
  applies a *render cutoff* (`setUiRenderCutoff`) so humans keep scrolling the full history
  while the model sees `[summary, …post-anchor]`.

## 7. Post-compaction side effects (r1462 `ps()` success path)

1. `resetPromptCacheSnapshot(sessionId)` — cache invalidated explicitly.
2. Drop any runtime **model-router pick** ("Compaction dropped routed pick; re-routing") and
   release refusal pins — the next request re-routes from scratch.
3. Append a `user_only` notice message (i18n `compaction.notice`) so the transcript shows the
   seam.
4. Persist `compaction_state` (above), emit `session_compacted` event
   `{summaryId, removedCount, visibleBoundaryMessageId}` to the UI.
5. Skills consolidation + discovery epoch (above).
6. Working state machine transitions `streaming_assistant_message → compacting_conversation →
   streaming_assistant_message` — compaction is *inside* the turn state machine, not a
   separate mode.

## 8. Settings surface (schema, r0924@54300)

```
compactionTokenLimit: number
compactionTokenLimitPerModel: { [modelId]: number }   // ≤ N entries, family-matched
compactionModel: string
compactionThresholdCheckEnabled: boolean
```
plus mission-level `missionModelSettings`, `subagentModelSettings` (subagents compact
independently with their own limits), and `llmRequestTimeout` / `subagentInactivityTimeout`.

## 9. What this means for oino

Droid's compaction is **iterative, anchored, append-only, and cache-aware**. The specific
ideas worth porting, in value order:

1. **Anchor + append-only `compaction_state` record in the session stream** — the summary is
   data, not a message mutation; every message tags its epoch (`compactionSummaryId`);
   reload = splice at anchor. UI shows everything, model sees the splice.
2. **Iterative delta summarization with soft cap** — never re-summarize from scratch;
   feed previous summary + delta; strip stale TODO blocks before feeding back; adaptive
   prompt shape over fixed sections.
3. **Exact trigger from real usage + suppression ref** — no compaction loop when usage won't
   drop; error-driven `context_limit` compaction that *retries the same turn* with
   `requireProgress`; threshold compaction failure is non-fatal.
4. **Summarizer window-fitting ladder** (25% instructions clamp, 4k reserve, pinned tail +
   omitted-count placeholder, 0.9→0.75→0.5 shrink ladder, resample/escalate).
5. **Cache-pin marker + reminder id stability** — compaction and reminder refresh both
   preserve provider prompt-cache prefixes; explicit `resetPromptCacheSnapshot` after
   compaction.
6. **Retention lists** (skills, images) and post-compaction discovery epoch — capability
   survives the cut instead of being rediscovered by accident.

Proven vs inferred: everything quoted above is **proven** from recovered code strings/log
lines (cited regions). Inferred only: exact semantics of `bo+Cn` in the usage formula
(assumed input+cache-read accounting) and the precise cache-pin protocol against each
provider.
