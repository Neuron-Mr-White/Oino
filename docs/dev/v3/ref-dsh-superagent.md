# How dsh-superagent works

Study notes on the research prototype at `/tmp/dsh-superagent` (read-only), written 2026-11 for oino.
Everything below is grounded in the repo; where docs claim something the code does differently (or not at
all), that is called out. Line numbers refer to the files as published.

---

## 1. One-paragraph summary

dsh-superagent is a **proactive coding agent for exactly one GitHub repository** (optionally also Feishu
chats), packaged as a plugin bundle for DeepSeek Harness (DSH). It watches the repo through the agent's
own GitHub account, turns selected events ("Triggers") into authorized work items ("Tickets") via a
deterministic policy gate plus an LLM assessment, executes each Ticket in its own DSH session and its own
git worktree directory, and answers in the original conversation — typically with a pull request plus a
short reply (`README.md:8`). The problem it solves is not "make an agent code" (DSH already does that) but
**making an agent a low-attention team member**: it acts only when asked or when a written rule allows it,
speaks only when a human must act or can use a result, and shows a panel that lists only what needs a
person (`docs/000-philosophy.md:5`). It is explicitly a research prototype, "not an official DeepSeek
product", and the README warns that some designs in the accompanying article *Exploring Super-agent
Design* are future direction, not implemented (`README.md:4-6`).

---

## 2. Domain model in plain language

Defined in `docs/001-domain.md` and implemented in `src/core/model.ts` (~353 lines of pure types +
predicates; the core imports nothing outside itself — enforced rule in `docs/007-architecture.md`
"Dependency rules").

| Concept | Plain meaning | Where it lives |
|---|---|---|
| **Source** | A stream of events, e.g. "the agent's GitHub account's view of repo X". A *kind* (github, feishu) is a plugin; an *instance* is one configuration. The Source owns its read cursor, drops the agent's own messages, and translates each event into Trigger + Thread + Channel address. | `src/sources/kind.ts`, `src/sources/github.ts`, `src/sources/feishu.ts` |
| **Trigger** | One immutable event (a comment, an issue, a CI transition) with the actor's identity, whether a human wrote it, whether it addressed the agent, and opaque Source metadata. **The unit of idempotence**: the same trigger id can be received twice, it stores once. | `src/core/model.ts:41-59`; dedupe in `src/core/intake.ts:39` |
| **Thread** | Triggers in time order — on GitHub one Issue/PR number (`github:123`), on Feishu a topic thread. At most one Eval runs per Thread at a time; it holds Subscriptions, the running Eval and Decision history. | `src/core/model.ts:170-196` |
| **Correlation** | The fact that two Threads share one Trigger (a comment mentioning two issues). Adds context only — never routes. Explicitly a "rejected concept" to let it route. | `docs/001-domain.md` "Rejected concepts"; `Thread.correlated` |
| **Channel** | The write side of a Thread: where replies go (issue conversation, review thread, Feishu topic). A Ticket holds a set. | `src/core/model.ts:27-28`, `src/core/replies.ts` |
| **Policy** | The versioned rules: who may originate work per Source (allowed lists in Source-native identities), an admission table per receiving scope, bot lists, proactive rules, and the accept mode (`off`/`observe`/`auto`). Version is a hash (`policyVersion`, `src/policy.ts:120-123`). | `src/policy.ts` |
| **Eval** | The judgement on a batch of queued Triggers, two layers: a **deterministic gate** (who is this, did they address the agent, what does the table say → explicit / inferred / ignore) and an **assessment** (the main session's LLM judging proactive candidates against enabled rules). Output is a **Decision**: accept, stop, ignore, or unavailable. Eval holds no state. | `src/core/evals.ts`, `src/policy.ts` `gate` |
| **Mandate** | What an accept grants: immutable record of policy version, strength (explicit/inferred), scope, matched rules, sender identity and admission basis. Only three things ever change it: accept creates it, a person's stop withdraws it, failed revalidation revokes it. Revalidated against *today's* policy before an ended Ticket resumes. | `src/core/model.ts:133-148`; revalidate in `src/policy.ts:200-241` |
| **Ticket** | A unit of authorized work. Only Eval creates one. Holds Channels, Inputs, Assignments and the Threads it follows. Open → ended; a person's message on one of its Threads resumes it (no new Eval) unless a person stopped it. Defines its own state, independent of the executor session. | `src/core/model.ts:264-330`, `src/core/tickets.ts` |
| **Assignment** | The binding of a Ticket to one Executor instance (pending → running → ended). A Ticket has several over time (retry, restart). Only a final report ends the Ticket on the executor's side. | `src/core/model.ts:220-232` |
| **Subscription** | Many-to-many Ticket↔Thread, held by the Thread. A matching Trigger fans out to every subscribed live Ticket with *no* gate check — context, not authority. A person's message that reaches a Ticket this way is **that Ticket's worker's to decide**, never re-evaluated. Ended Tickets keep subscriptions so later human comments resume them. | `src/core/intake.ts:44-66`, `docs/001-domain.md` Subscription row |

Deliberately **not** concepts (docs/001 "Rejected concepts"): person/principal entities, cross-Source
identity mapping (prompt text instead), waiting states in Eval (waiting belongs to Sources), one-ticket-per-thread, "append to Ticket" decisions (that's fan-out).

### Mermaid: a GitHub @mention becoming a PR reply

```mermaid
flowchart TB
  A["Human comment on Issue #12:\n@agent-login fix the typo in settings.md"] -->|poll (60s)| S[GitHub Source\nnotifications + subscribed threads]
  S -->|Trigger: human, addressed=mention,\nstable id incl. updated_at| I[Intake]
  I -->|store once + fan-out\nIssue thread not subscribed yet| T[Thread github:12]
  T -->|batch queued Triggers| E[Eval on Thread]
  E -->|gate: sender on allowed list\nAND mentions agent| G[strength = explicit]
  G -->|candidate card steered\ninto main session| M[Main Session\norchestrator_route: accept]
  M -->|atomic: Decision + Ticket\n+ Mandate + Subscription\n+ first Assignment + ack| K[Ticket]
  K -->|ack reply queued| C[Channel: issue comment\n"This is by an automated agent... On it: ..."]
  K -->|launch effect| X[DshExecutor]
  X -->|prepare: git worktree\nagent/&lt;name&gt;-&lt;ticketid12&gt;| W[Worker session worker-&lt;ticketId&gt;\nin its own directory]
  W -->|implement, commit, push,\nopen PR, orchestrator_report\nready + pr_number| B[bindThread: PR thread]
  B -->|reply effect via gh| R[PR link posted on Issue #12]
  B -->|PR merged/closed observed\nby Source poll| F[Ticket ends merged;\nsubscriptions kept for later resume]
```

---

## 3. End-to-end flow of one piece of work

### 3.1 @mention → admission/eval

1. **Poll**: the GitHub plugin polls the agent account's notifications plus every subscribed Issue/PR
   (with CI) each round; default `pollSeconds: 60` (`src/sources/github.ts:29-33`, watch loop at :146-160).
   Edits produce new events because the comment Trigger id includes `updated_at`
   (`src/sources/github-trigger.ts:79-81`); a mention added by *editing an issue body* is not received
   (docs/006 "Try it" note) — only new comments/issues mention reliably.
2. **Trigger**: `commentTrigger` marks `human: true` (unless `[bot]`/App), `addressed: 'mention'` if the
   text matches `@<agent-login>` (`src/sources/github-trigger.ts:88-102`), attaches the Channel
   (issue number + comment URL).
3. **Intake** (`src/core/intake.ts:35-79`): dedupe by id → `duplicate`; Source scope filter → `filtered`
   with reason; else store, update Thread, fan out to subscribed tickets (a *human* message on a
   non-context thread resumes an ended non-human-stopped ticket immediately), then **queue for Eval on
   the primary Thread unless some ticket's worker owns it** (`followupOwners`, `src/core/model.ts:87-108`).
4. **Eval** (`src/core/evals.ts:87-141`): one Eval per Thread; triggers arriving meanwhile batch into the
   next one. The gate (`src/policy.ts` `gate`, :169+) computes the strongest admission:
   - allowed sender + addresses agent → **explicit**;
   - follow-up on a thread with tickets → explicit (listed) or inferred (unlisted);
   - admission-table cells (3 per receiving scope: listed-unaddressed / unlisted-addressed /
     unlisted-unaddressed, defaults in `src/admission.ts:13`) → explicit/inferred/ignore;
   - bots: only listed bots, inferred only, and never in `off` mode (`machineGate`, `src/policy.ts:127-151`).
5. **Candidate → main session**: an `assessment` effect steers a candidate card into the main session's
   turn; the deadline (default 120s) counts from *delivery*, and expiry re-queues twice before recording
   `unavailable` (`src/core/evals.ts:161-186`).
6. **Decision**: the main session routes with `orchestrator_route` (batch of 1-20 decisions, each an
   independent Engine command, `src/app/tools/main.ts:44-133`). Explicit candidates can be accepted
   directly; inferred ones *must* go through `orchestrator_eval`/`orchestrator_eval_decide`, and
   acceptance requires settled direction + ≥1 matched enabled rule + `intent_evidence`
   (`assessmentOutcome`, `src/policy.ts:47-76`; in non-auto mode a positive assessment is recorded as
   ignore "would accept"). **Accept is atomic**: Decision + Ticket + Mandate + Subscriptions + first
   Assignment + acknowledgement receipt in one transaction (`src/core/tickets.ts:44-76`, tested in
   `tests/core-engine.spec.ts:136`).

### 3.2 Ticket → worker session → PR + reply

- `prepareTicket` stamps execution metadata: worktree name `<english-name>-<ticketid12>`, directory under
  the execution root (default `<stateDir>/work`), assignment text (`src/executor/dsh.ts:98-113`).
- A `launch` effect fires: reserves a worker slot (concurrency `maxWorkers`, default 3; +1 overflow slot
  reserved for human messages so a person never waits behind launches — `WorkerSlots`,
  `src/executor/dsh.ts:199-244`), runs the **prepare command** once with `sh -c` in the directory's parent
  with the execution env vars (`DSH_SUPERAGENT_GITHUB_REPOSITORY`, `_NUMBER`, `_PR_HEAD`, `_DIR`, …) and a
  10-minute timeout (`src/executor/dsh.ts:264-296`). The documented prepare command fetches
  `refs/dsh-superagent/$DSH_SUPERAGENT_TICKET` and `git worktree add -b agent/$DSH_SUPERAGENT_NAME`
  (docs/006 "Settings" §3). A failed prepare fails the ticket (no auto-retry — retrying would treat the
  half-written directory as prepared; `PrepareFailed`, `src/executor/dsh.ts:37, 296-323`).
- The executor creates/resolves session `worker-<ticketId>` with the `dsh-superagent-worker` preset and
  the ticket directory as workspace (`src/app/sessions.ts:155-175`), then delivers the assignment message
  (title, ≤1500 chars assignment text, origin trigger card, batch lines) via `AgentDelivery` — exactly
  once per request id, across restarts (`src/app/delivery.ts`).
- The worker (prompt `prompts/worker.md` + `prompts/work.md` + skills) reads `AGENTS.md`, investigates,
  implements, verifies, commits as the agent account (git config has no identity; env
  `DSH_SUPERAGENT_GITHUB_ACCOUNT`/`_COMMIT_EMAIL`), pushes branch `agent/<name>`, opens a PR with `gh`,
  and immediately reports `orchestrator_report progress` with `pr_number` — this *binds* the PR thread to
  the ticket (`claimPull`/`observePull`, `src/sources/github-pull.ts`; skill
  `skills/dsh-superagent-github/SKILL.md`).
- Ticket end follows the PR lifecycle, not the report: `ready` + `followsDone` (no PR it opened still
  open; or all followed threads closed) → ends `merged`/`closed` when the Source reports
  `threadClosed` (`src/core/tickets.ts:105-141`, `src/core/assignments.ts:141-155`). "Awaiting review" =
  a `ready` report with an open PR.
- Replies go through the Engine's `Replies` rules (`src/core/replies.ts`): acknowledgement only for a
  person's message that reached the ticket; `result` only when ready/completed/merged/dropped;
  `attention` must quote the ticket's current structured question verbatim. The **Outbox**
  (`src/app/outbox.ts`) persists a receipt per message id, sends via the Source's channel (`gh api ...`),
  keeps per-ticket per-conversation ordering, retries 5s→300s, and settles `channelGone`/`channelIgnored`
  when the conversation can never receive it. GitHub sends are idempotent via a hidden comment marker
  (docs/008 §5).

### 3.3 Follow-up comments

A later comment on the issue/PR (no @mention needed) is fanned out to the subscribed ticket as an Input
**without any Eval** (`src/core/intake.ts:70-71`); the main session only gets a `follow_up` notice whose
note says "act only if the person asks to stop" (`src/app/notices.ts:74-95`). The worker decides: in-scope
→ acknowledge (`orchestrator_ack`) and do it; beyond scope or for another ticket → hand back once
(`orchestrator_trigger_handback`), which queues a fresh Eval once no other ticket owns it
(`src/core/evals.ts:289-306`). When a ticket ends, follow-ups its worker never *read* (read = taken into a
session step + followed by a report/turn-end, `src/core/assignments.ts:56-68`) are released back to Eval
(`Tickets.letGo`, `src/core/tickets.ts:222-260`). If the ticket had already ended, **any person's message
on its threads resumes it under the same Mandate with no Eval and no Decision** — after revalidating the
Mandate against the current policy (`Tickets.wake`, `src/core/tickets.ts:262-288`; bots/CI never wake it).
Cost discipline is tested: "costs the main Session one model call for a burst of follow-ups while it
works" (`tests/followups.spec.ts:103`).

### 3.4 "stop"

- A person saying "stop" in a thread is an **ordinary Trigger through Eval**; a stop Decision requires
  `explicit` strength (else recorded ignore `stopNotExplicit`, `src/core/evals.ts:258-266`). An executor
  can never declare that a person asked it to stop.
- Panel **Stop** is a direct command (`stopTicket`) and records a manual stop; both end the ticket `by:
  human` and **withdraw the Mandate** — the ticket receives nothing more; new work needs a new accept
  (`src/core/tickets.ts:172-177`, `src/core/engine.ts:151-153`).

### 3.5 Policy changes

`policyVersion` hashes rules + accept mode + per-Source policy parts (`src/policy.ts:120-123`). On
change: running Evals are discarded and their Triggers re-queued under the new version
(`Evals.policyChanged` → `reassess`, `src/core/evals.ts:308-316`); each *affected* live ticket gets a
`policy-changed` Input saying exactly what changed (rule changed/disabled, requester no longer allowed,
proactive no longer accepted — `policyEffects`, `src/policy.ts:161-168`); running tickets are **not
cancelled** (docs/001 rejected concept). A decision made against a stale version is recorded
`unavailable` and re-queued (`src/core/evals.ts:252-256`).

---

## 4. LLM-facing design

### Roles and prompt assembly

Two DSH roles, both created with dedicated presets defined in `cordis.patch.yml`:

- **Main session / orchestrator** (`prompts/orchestrator.md`, `dispatch.md`, `subscriptions.md`,
  `replies.md`, `admission.md`, `cli.md`, `handoff*.md`): *only routes*. It forwards original triggers +
  routing facts, makes no technical decisions, never inspects code outside an assessment, never waits
  (`noWait` guard refuses `sleep`, shell timeouts >30s, job waits >5s — `src/app/sessions.ts:58-85`).
  Restricted to 7 DSH tools + 14 app tools (`src/app/sessions.ts:262-263`).
- **Worker** (`prompts/worker.md`, `work.md`, plus the repo's `AGENTS.md` and the DSH coding prompt):
  owns investigation, scope judgement, implementation, verification, review handling, and follow-up
  decisions. Keeps the profile's work tools (`WORK_TOOLS`, `src/app/sessions.ts:56-70`); loses tools the
  moment its ticket ends (`workOnly` guard, :276-296); subagents inherit the same limits (:297-315).

Assembly order (docs/004 "Assembly"): shared+role → 3 team sections (Communication / Ticket scope /
Delivery requirements, `prompts/team/`) → subscriptions/replies → dispatch-or-work → admission (main
only) → handoff → environment → policy (main only, regenerated per model request so a policy change
leaves earlier prompt bytes identical). Precedence ladder: host-enforced facts > built-in/generated
sections > matched-rule instructions for that ticket > team sections/skills > outside text (never
authority). Every prompt line is one sentence; `promptFile` joins them (`src/app/skill-files.ts`).

### Executor tools (worker, 6 app tools)

`orchestrator_read` (its own ticket's resources only), `orchestrator_ack`, `orchestrator_report`
(progress/waiting/ready/blocked/failed/dropped + optional `human_action` question + `reply_to_trigger_id`
answers + `pr_number` binding), `orchestrator_thread_unrelated`, `orchestrator_thread_follow` (context-only
subscription), `orchestrator_trigger_handback` (docs/007 "Executor tools").

### How it avoids polling / human attention

- **Background-first shell**: both presets move a foreground command to a background job after **5
  seconds** (`cordis.patch.yml` `timeoutMs: 5000`), and job waits cap at 5s; a finished job wakes its
  session with its result. Workers are told "do not poll; end your turn" (`prompts/work.md`).
- **Steering vs injection**: candidates, follow-ups, worker reports and job results are *steered* into
  the main session's turn (wake it); bookkeeping notices are *injected* without waking
  (`Sessions.prompt` modes, `src/app/sessions.ts:186-215`).
- **Structured attention**: "needs a human" is a structured field (`human_action` with
  `question_for_trigger_id`), never inferred from free text (docs/001 rejected concept); the orchestrator
  forwards it verbatim to the channel where the question was raised. Panel stage `person/answer` comes
  from `ticket.attention`, `person/review` from a `ready` report — the classification table is
  checkable and mutually exclusive (`src/core/stage.ts:12-27`, row-by-row test `tests/stage.spec.ts`).
- **Quiet-by-default reporting**: progress reports stay internal (no message); one-sentence chat
  messages with a hard 100-word ceiling enforced by the transport (`prompts/replies.md`,
  `checkedImText` in `src/app/outbox.ts:41`); acknowledgements once per trigger, deduped by receipt id.

### Retries / health

- Transient provider/network failures (408/429/5xx, ECONN*, etc. — regex `PASSING`,
  `src/executor/dsh.ts:39-48`) are retried 3× with backoff: launches via the effect queue (5s→300s),
  failed worker turns get a "continue where it stopped" message after 15/30/60s
  (`resumeAfter`, :585-610). Non-transient failures record a `failed` report; the ticket then waits for a
  person (automatic inputs held — `waitsForPerson`, `src/core/model.ts:346-353`) or a retry.
- Background work after a turn gets 30 minutes, then jobs are killed (subagents allowed to continue) and
  the ticket reports `waiting` with the worker's last words (`workerIdle`, `src/executor/dsh.ts:662-673`).
- Every subsystem claims one health entry on a `HealthBoard` (one writer per entry); effects report error
  after 3 consecutive failures; `scripts/dsh-superagent status` exits 0/1/2/3 and an *hourly check* agent
  in the operator's own profile restarts the service (`--respect-stop`) or reports errors — self-healing
  without paging on noise (docs/006 "Hourly check").

### Model routing

No automatic routing or fallback: the profile's default model, unless `eval.model` (main session) or
`executor.model` (workers) names one (`RoleModelSchema`, `src/policy.ts:81`; `src/app/role-models.ts`).
A session keeps the model it was created with. Model-budget retries surface as the `models` health entry
and as `model_retry` on ticket cards (`src/app/retry-health.ts`).

---

## 5. The web panel

`src/client/` (React, served on `127.0.0.1`, default port `3082` via the profile's `webserver` entry;
"Superagent" sidebar entry). It answers three questions in order (docs/005): (1) does anything need me
now, (2) is the system healthy, (3) what is the history of one piece of work. Tabs mirror the domain:
**Overview** (tickets needing action, system problems, 24h activity counts, executor load), **Source**
(state, filtered triggers, dropped messages), **Trigger** (threads, newest triggers, decisions,
subscriptions), **Eval** (decisions with reasons and matched rules), **Execution** (tickets by stage,
pause processing, retry/stop, waiting inputs, assignments), **Settings** (Sources / Eval policy +
admission tables / Execution / Prompts with versioned history and "Default updated" diffs / Deployment).

A human is needed exactly when a ticket's stage is a `person/*` stage: input undelivered, awaiting
answer (an Attention), awaiting review (a `ready` report / open PR), blocked/failed with no human message
since, plus system problems (effect kinds failing 3×, source poll failures, unresolved `unavailable`
Decisions <24h old). Healthy state takes no space. Panel-only actions (deliberately no tool):
dismissing an unavailable decision, rerunning setup, sending an executor-owned follow-up to Eval
(docs/007 "Panel-only actions"). The management API behind the panel (`/orchestrator/api/*` with
`x-orchestrator-request: 1`, loopback/same-origin enforced, `src/app/http.ts`) is also what SETUP.md's
agent uses to save settings.

---

## 6. Persistence and state

- **One SQLite file** `<stateDir>/orchestrator.sqlite`, WAL, opened by exactly one process (a
  `proper-lockfile` lock on the state dir is taken *before* opening, `src/app/runtime.ts:96-100`).
  Tables: `trigger`, `disposition`, `trigger_thread`, `thread`, `ticket`, `decision`, `effect`, `outbox`,
  `kv` — each row a JSON blob plus generated columns (`active`, `open`, `subscribed`) with indexes so a
  pass reads only live work (`src/app/store.ts:64-110`). Schema 11 only; older (10) migrated once.
- **One writer per state kind** (docs/007): Engine writes all domain aggregates; Sources write only their
  cursors and Thread `meta`; Effects owns the effect table; Outbox owns receipts; Settings/Prompts own
  versioned snapshots in `kv` (typed key registry `src/kv.ts` — every key spelled once, enforced by
  `tests/store.spec.ts`).
- **All Engine methods are synchronous and transactional**; ports enqueue external work *inside* the
  transaction, adapters perform it *after commit* (`afterCommit`). This is the crash-safety trick: a
  rolled-back accept never launches a worker or sends an ack.
- **Effects** = durable side-effect queue (`src/app/effects.ts`): kinds `assessment`, `launch`,
  `delivery`, `relay`, `notice`, `reply`; one row per kind+key (re-enqueue coalesces, keeps queue
  position, resets attempts); exponential backoff; `Later` defers without counting a failure (pause, no
  slot, source off); relay/notice are ordered, a failed one holds back later ones of its kind.
- **Outbox receipts** make channel sends idempotent (same id never queued twice; dropped receipts keep
  when/why and are never retried).
- **Restart**: runtime shutdown cancels turns, drains effects, waits ≤10s per worker's background work.
  On startup, running Assignments are reported `assignmentInterrupted`; one that was mid-work restarts
  with a recovery note ("inspect worktree, git history and GitHub before repeating any outside action",
  `src/core/assignments.ts:20-25`); pending launches re-queue oldest-first; inputs not yet taken into
  session history are re-delivered exactly once (a stale copy still waiting in the session is removed
  first, `src/executor/dsh.ts:520-526`). Which messages a worker *read* is durable
  (`inputsRead` on `session/event user/message`, :570-584), so end-of-ticket follow-up release survives
  crashes.
- **Idempotence everywhere**: trigger ids deterministic from source ids (GitHub comment id + `updated_at`;
  re-reads are safe and cursors only advance after every covered trigger is stored); accept idempotent by
  trigger; acknowledgement deduped by `key('ack', trigger.id)`; reply texts keyed deterministically.
- Known gap: **nothing deletes old records** — triggers, dispositions, decisions, ended tickets, receipts
  and every settings/prompt revision grow without bound (`docs/007-architecture.md:484`).

---

## 7. Deployment inside DeepSeek Harness

- **Bundle layout**: a DSH plugin package (`package.json` `dsh.bundle` manifest, `manifestVersion: 1`) with
  three built entries — `index` (the orchestrator Runtime), `github-watch`, `feishu-watch` (Source
  plugins, each `inject: ['orchestrator']` and registering its kind on the `SourceHost` service,
  `src/sources/github.ts:137-152`). `cordis.patch.yml` (the bundle's patch layer) *inserts* the three
  plugins (Feishu `disabled: true` by default) **plus two agent presets**, `dsh-superagent-worker` and
  `dsh-superagent-router`, built from DSH's own plugin blocks: 5s shell/job timeouts, no ask-user /
  present / plan / plugin-manager / scheduling for workers, and for the router additionally no subagents,
  web, or file writes.
- **Profile**: a dedicated `superagent` profile (copied from the Web profile) so the plugin's
  process-wide `PATH` and git changes never touch the operator's own sessions;
  `examples/profile.cordis.patch.yml` supplies `stateDir` and the webserver port. Machine config beyond
  `stateDir` (prompt size cap, admission context page/time limits) is volatile and hot-reloaded.
- **Scripts**: `scripts/dsh-superagent start|stop|restart|status` runs `dsh --profile superagent` inside a
  tmux session, refuses duplicate instances, exits 0/1/2/3 as described above, keeps the newest 20 logs
  under `.artifacts/`. `--respect-stop` keeps an intentional stop.
- **"One repo" scoping**: the GitHub config names exactly one `owner/name`; the Source scope receives
  everything in that repository only when the four `observe*` switches are on, elsewhere only what
  addresses the agent (`scope` in `src/sources/github.ts:56-60`); `gh` CLI calls are validated and
  limited to that repository (`src/infra/commands.ts` `api`/`pages`); search `find` maps `#12`/URLs of
  that repo only. GHES unsupported.
- **Credential hygiene**: setup writes `<stateDir>/worker-bin` shims first on the worker `PATH` (`gh` →
  the agent-account wrapper; blocked commands fail) and `<stateDir>/worker-gitconfig` (agent CLI as the
  github.com credential helper, SSH→HTTPS rewrite, no commit identity, `GH_TOKEN` stripped); env vars
  matching KEY/PASSWORD/SECRET/TOKEN are removed; a foreign file in those directories stops setup
  (docs/006 "Setup checks"). Explicitly *defaults, not a security boundary* — workers run
  `danger-full-access` as the operator's OS user; isolation requires a dedicated OS account.

---

## 8. What the code does NOT implement (docs/article frame as future direction)

The README states the prototype is an adaptation and that "some designs discussed in the article are
directions for future development and are not implemented" (`README.md:4-6`). The article itself is not
in the repo; from repo evidence, these described/claimed directions are absent:

1. **Waiting-condition Sources** — docs/002 says a condition like "no assignee for N hours" *would be* a
   Source that waits and rechecks; no such Source exists (only GitHub, Feishu; the cookbook's `webhook`
   kind is documentation-only, docs/008).
2. **Team/company scale** — exactly one repository, two Sources, one operator; cross-Source identity
   mapping is explicitly a rejected concept (prompt text at best).
3. **Retention/compaction of history** — nothing deletes old records yet (docs/007:484); the internal
   version presumably had pruning.
4. **Deployment isolation** — the open question of pinning the agent's own DSH installation
   (docs/007 "Open questions"), plus the trust model for Trigger fields, is unresolved.
5. **True sandboxing** — `danger-full-access` as the operator's user; wrappers are "defaults, not a
   security boundary" (docs/003 "Execution variables", docs/006 "Permissions and isolation").
6. **Domain events without consumers** — `trigger.followup`, `eval.started` etc. are emitted but "other
   events currently have no consumer" (docs/007 Runtime section); some analytics/audit surface is implied
   but absent.
7. **Panel/ops parity gaps** — dismissing unavailable decisions, rerunning setup, and re-evaluating
   executor-owned follow-ups exist only in the panel, with no tool (deliberate, but limits full-agent
   self-management).

---

## 9. If oino wanted this

oino already has the hard execution substrates: a Rust agent core with sessions/tools/skills, WASM
extensions as the extension model (`extensions/unipi/*`), and `unipi.long-horizon` with durable agent
graphs (`/goal`, `/ralph`, `/swarm`, `/graph`, docs/unipi.md:13-52), plus `unipi.background-tasks`
(bg_run with wake-on-finish), `unipi.notify`, `unipi.subagents`, and `unipi.workflow`'s permission gate.
dsh-superagent's value is almost entirely the **authorization, routing and attention layer in front of**
execution — which oino currently lacks. Mapping:

- **Source/Trigger → WASM extension**: a `github-watch` equivalent is a WASM component implementing a
  `source` interface (scope, translate-to-trigger, channel.send, thread-context reads — exactly the
  ~9-hook `SourceKind` protocol of docs/008). oino's `oino-extension-api` WIT could grow `trigger`,
  `channel` and `cursor` resources; the host owns durable cursors (the cookbook's contract: stable ids,
  cursor advances only after storage, idempotent sends, `gone` vs throw).
- **Eval/gate + Ticket/Mandate → admission in front of the agent graph**: a Ticket is a graph node with
  an immutable Mandate payload; `Assignment`s are node runs; `followsDone` (PR merged/closed) is the
  node's completion predicate supplied by a Source rather than by the model. The durable graph gives
  oino what dsh-superagent rebuilds with SQLite tables; the Mandate/gate gives the graph what it lacks:
  host-enforced, auditable authorization per node.
- **Subscription fan-out → graph edges + oino `topics`**: `oino-core` already has `topics.rs`;
  thread-subscription fan-out is a topic bus where a *human* message addressed to a running node's topic
  is delivered to that node's agent (steer) instead of spawning a new graph node — the single biggest
  token saver in dsh-superagent (one model call per burst of follow-ups, `tests/followups.spec.ts:103`).
- **Effects/Outbox → graph persistence**: oino's long-horizon engine needs exactly the after-commit
  effect queue (coalescing kind+key, `Later` deferral, ordered relays) and idempotent receipts if it ever
  replies to external channels.
- **Stage table + attention → unipi tray/notify**: `ticketStage`'s checkable "who is this waiting for"
  classification and the structured `human_action` question (never inferred from free text) would make
  unipi's ask-user cards and notify events honest about *why* a human is needed.

**The 5 most valuable ideas worth adopting regardless:**

1. **Host-enforced Mandates** (immutable "why may this work run" recorded per work item; only
   accept/stop/revalidation change it; prompt text and message content can never alter it). Even for a
   single-user agent this gives auditable scope per long-horizon goal and a revalidation hook when
   permission settings change.
2. **Subscription fan-out with worker-owned follow-ups and hand-back** — later messages reach the
   running worker directly (no re-evaluation, no re-mention), the worker hands back what is out of
   scope, and only then does it become new work. Eliminates the re-LLM-per-message tax and duplicate
   tickets.
3. **The durable after-commit effect queue + idempotent outbox** (coalescing, backoff, `Later`,
   ordered relays, receipts with `gone` vs retry). Directly reusable as the persistence spine of oino's
   graph engine and any channel integration.
4. **"The session never waits" discipline**: 5s foreground→background, no `sleep` (guard), steering for
   what wakes the model vs injection for what must not, finished jobs waking their session. oino's
   bg-tasks already wakes; the *policy* (guard + prompts + preset caps) is the missing half.
5. **observe → auto rollout for autonomous behavior**: proactive candidates are shadow-evaluated in
   `observe` mode and recorded as "would accept" Decisions with full audit (matched rules, direction,
   intent evidence) before the team flips to `auto`. A generic, safe adoption path for any oino feature
   that lets the agent act unprompted (cron-like schedules, proactive fixes) — note `/cron` was dropped
   from unipi; this is the pattern that would let it return safely.

Honorable mentions: the checkable ticket-stage table for triage UIs; the prompt-precedence ladder
(host facts > built-ins > rules > team text > outside data); "attention costs" as an explicit design
budget for every message/notification/setting.
