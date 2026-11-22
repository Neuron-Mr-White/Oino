import { z } from 'zod';
import { hostCall, hostClient } from '../../lib/host/factory';
import type { HostClient } from '../../lib/host/client';
import { toAppError } from '@unipi/protocol';

export const AutonomySchema = z.enum(['low', 'medium', 'high']);
export const MODES = ['none', 'goal', 'ralph', 'swarm', 'graph'] as const;
export type LeadMode = (typeof MODES)[number];
export const REVIEWS = ['none', 'light', 'full'] as const;
export const PAUSED_GRAPH_POLICIES = ['report', 'reuse'] as const;
export type PausedGraphPolicy = (typeof PAUSED_GRAPH_POLICIES)[number];
export const PausedGraphPolicySchema = z.enum(PAUSED_GRAPH_POLICIES);

const ProgressSchema = z.object({ running: z.number().default(0), queued: z.number().default(0), done: z.number().default(0) });
const PausedGraphSchema = z.object({ sessionId: z.string().min(1), title: z.string().nullish() });
export type PausedGraph = z.infer<typeof PausedGraphSchema>;

/** A lead's life cycle (OIN-80). A host that predates it sends no `state`: the lead is working. */
export const LEAD_STATES = ['working', 'waiting', 'idle', 'unloaded', 'stalled'] as const;
export type LeadState = (typeof LEAD_STATES)[number];
/** Why a stalled lead stopped. */
export const HEALTH_KINDS = ['model', 'no_progress', 'launch'] as const;
const parseEpochMs = (v: number | string): number | null => {
  if (typeof v === 'number') return Number.isFinite(v) ? v : null;
  const trimmed = v.trim();
  if (!trimmed) return null;
  const parsed = Date.parse(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
};
const optionalMs = z.union([z.number(), z.string()]).transform(parseEpochMs).nullish().catch(null);
const LeadHealthSchema = z.object({
  kind: z.enum(HEALTH_KINDS).nullish().catch(null),
  error: z.string().default(''), attempts: z.number().default(0),
  /** Epoch ms of the first failure. */
  since: optionalMs,
  /** Epoch ms of the next automatic re-probe; absent when none is scheduled. */
  nextProbeAt: optionalMs,
});
export type LeadHealth = z.infer<typeof LeadHealthSchema>;
const LeadQuestionSchema = z.object({ taskId: z.string(), excerpt: z.string().default('') });
const LeadSchema = z.object({
  projectId: z.string(), sessionId: z.string(),
  presetId: z.string().nullish(), model: z.string().nullish(), thinking: z.string().nullish(),
  autonomy: AutonomySchema, mode: z.string().nullish(),
  ticketIds: z.array(z.string()).default([]),
  step: z.string().nullish(), tool: z.string().nullish(), startedAt: z.string().nullish(), cost: z.number().nullish(),
  progress: ProgressSchema.nullish(),
  pausedGraph: PausedGraphSchema.nullish(),
  /** OIN-80: all optional, an older host omits them. */
  state: z.enum(LEAD_STATES).optional().catch(undefined),
  health: LeadHealthSchema.nullish().catch(null),
  question: LeadQuestionSchema.nullish().catch(null),
  idleReason: z.string().nullish().catch(null),
  lastActiveAt: optionalMs,
});
/** An older host reports one worker per ticket. */
const WorkerSchema = z.object({
  projectId: z.string(), taskId: z.string(), sessionId: z.string(), autonomy: AutonomySchema,
  step: z.string().nullish(), tool: z.string().nullish(), startedAt: z.string().nullish(), cost: z.number().nullish(),
});
const NeedsSchema = z.object({
  projectId: z.string(), taskId: z.string(), status: z.string(), question: z.string().nullish(),
  workerSessionId: z.string().optional(), requestId: z.string().optional(), sessionId: z.string().optional(), tool: z.string().optional(),
  command: z.string().nullish(), summary: z.string().nullish(), risk: AutonomySchema.nullish(), ceiling: AutonomySchema.optional(), reason: z.string().optional(),
  /** OIN-81: `question` is a lead's own ask_user (`in_progress`) or claim-held blocked question; epoch ms (ISO string from older host parsed). */
  kind: z.string().optional(), options: z.array(z.array(z.string())).optional().catch(undefined),
  askedAt: optionalMs, expiresAt: optionalMs, timeoutAt: optionalMs,
}).transform(({ timeoutAt, expiresAt, ...rest }) => ({
  ...rest,
  ...(expiresAt !== undefined || timeoutAt !== undefined ? { expiresAt: expiresAt ?? timeoutAt } : {}),
}));
const ReviewSchema = z.object({ projectId: z.string(), taskId: z.string(), status: z.string().optional() });
export const BLOCKER_CODES = ['not_registered', 'archived', 'project_auto_off', 'global_auto_off'] as const;
export const ENABLE_ACTIONS = ['register', 'unarchive', 'enable_project_auto', 'enable_auto_start'] as const;
export type EnableAction = (typeof ENABLE_ACTIONS)[number];
/** Why a project cannot take work, and the one action that clears it. */
const BlockerSchema = z.object({
  code: z.enum(BLOCKER_CODES), message: z.string(),
  action: z.object({ kind: z.enum(ENABLE_ACTIONS), label: z.string() }),
});
export type Blocker = z.infer<typeof BlockerSchema>;
/** `not_registered` and `archived` refuse any start; the auto blockers only stop automatic starts. */
export const isHardBlocker = (b: Blocker | undefined) => b?.code === 'not_registered' || b?.code === 'archived';
const ProjectSchema = z.object({
  projectId: z.string(), archived: z.boolean().optional(), name: z.string().nullish(), autoStart: z.boolean(), lead: z.string().nullish(), ready: z.number().int().nonnegative().default(0),
  root: z.string().nullish(), leadModel: z.string().nullish(), leadThinking: z.string().nullish(),
  /** Older hosts omit these: no blockers, always registered. */
  blockers: z.array(BlockerSchema).default([]), registered: z.boolean().default(true), path: z.string().nullish(),
});

/** OIN-82: something the host did that the owner should see once. Only `board_created` is drawn; unknown kinds are dropped by the reader. */
const noticeAtMs = z.union([z.number(), z.string()]).transform((v) => parseEpochMs(v) ?? undefined).optional().catch(undefined);
export const NoticeSchema = z.object({
  kind: z.string(), projectId: z.string().optional(), name: z.string().nullish(), at: noticeAtMs, text: z.string().optional(),
});
export type StatusNotice = z.infer<typeof NoticeSchema>;

/** OIN-80 Settings table: labels, ranges and help texts are the spec's, verbatim. */
export const LEAD_SETTING_FIELDS = {
  retries: { label: 'Lead retries before fallback', min: 0, max: 10, def: 3, help: 'How many times a lead re-sends its own turn after a provider error (429, 5xx, network) before it switches to the fallback model. This is separate from Graph auto-continue.' },
  fallbackModel: { label: 'Lead fallback model', help: 'Used when a lead runs out of retries. Empty means your default model.' },
  warmMinutes: { label: 'Keep idle leads loaded for (minutes)', min: 5, max: 1440, def: 30, help: 'Leads idle longer than this are unloaded. New work reopens the same chat.' },
  questionTimeoutMinutes: { label: 'Wait for your answer to a lead question (minutes)', min: 1, max: 1440, def: 30, help: 'After this the question becomes a blocked ticket. The lead keeps the ticket.' },
} as const;
const F = LEAD_SETTING_FIELDS;
const range = (key: 'retries' | 'warmMinutes' | 'questionTimeoutMinutes') => z.number({ error: 'Enter a number.' }).int('Use a whole number.')
  .min(F[key].min, `${F[key].min} to ${F[key].max}.`).max(F[key].max, `${F[key].min} to ${F[key].max}.`);
/** The lead settings; every key falls back to its default so an older or partial payload still parses. `fallbackModel` is `provider/id`, or null for "your default model". */
export const LeadSettingsSchema = z.object({
  retries: range('retries').default(F.retries.def),
  fallbackModel: z.string().trim().nullish().transform((v) => v || null).refine((v) => v === null || /^[^/\s]+\/\S+$/.test(v), 'Use provider/model, or leave empty.'),
  warmMinutes: range('warmMinutes').default(F.warmMinutes.def),
  questionTimeoutMinutes: range('questionTimeoutMinutes').default(F.questionTimeoutMinutes.def),
});
export type LeadSettings = z.infer<typeof LeadSettingsSchema>;
export const LEAD_DEFAULTS: LeadSettings = { retries: F.retries.def, fallbackModel: null, warmMinutes: F.warmMinutes.def, questionTimeoutMinutes: F.questionTimeoutMinutes.def };
/** The stored settings, tolerant: a bad key falls back to its default instead of failing the whole status. */
const readLeadSettings = (raw: unknown): LeadSettings => {
  const value = raw && typeof raw === 'object' ? raw as Record<string, unknown> : {};
  const pick = <K extends keyof LeadSettings>(key: K) => (LeadSettingsSchema.shape[key].safeParse(value[key]).success ? value[key] : undefined);
  const parsed = LeadSettingsSchema.safeParse({ retries: pick('retries'), fallbackModel: pick('fallbackModel'), warmMinutes: pick('warmMinutes'), questionTimeoutMinutes: pick('questionTimeoutMinutes') });
  return parsed.success ? parsed.data : LEAD_DEFAULTS;
};

const RawStatusSchema = z.object({
  enabled: z.boolean(),
  pausedGraphPolicy: PausedGraphPolicySchema.default('report'),
  leads: z.array(LeadSchema).optional(), workers: z.array(WorkerSchema).optional(),
  needsYou: z.array(NeedsSchema).default([]), review: z.array(ReviewSchema).optional(), projects: z.array(ProjectSchema).optional(),
  notices: z.array(NoticeSchema).catch([]).default([]), lead: z.unknown().optional(),
});

export type Lead = z.infer<typeof LeadSchema> & { legacy?: boolean };
export type NeedsItem = z.infer<typeof NeedsSchema>;
export type ApprovalItem = NeedsItem & { workerSessionId: string; requestId: string; sessionId: string; tool: string; ceiling: z.infer<typeof AutonomySchema>; reason: string };
export type ReviewItem = z.infer<typeof ReviewSchema>;
/** `blockers` / `registered` are always set on parsed rows; optional here so a row built by hand (an older host's shape) still type-checks. */
export type ProjectRow = Omit<z.infer<typeof ProjectSchema>, 'blockers' | 'registered'> & { blockers?: Blocker[]; registered?: boolean };
export type SuperagentStatus = {
  /** Optional so a status built by hand (an older host's shape) still type-checks; the reader always sets them. */
  notices?: StatusNotice[]; leadSettings?: LeadSettings;
  enabled: boolean; pausedGraphPolicy: PausedGraphPolicy; leads: Lead[]; needsYou: NeedsItem[]; review: ReviewItem[];
  /** null: the host predates per-project controls. */
  projects: ProjectRow[] | null;
  /** The host still reports one worker per ticket. */
  legacy: boolean;
};

export function isApproval(item: NeedsItem): item is ApprovalItem {
  return !!(item.requestId && item.workerSessionId && item.sessionId && item.tool && item.ceiling && item.reason);
}
/** A lead's own question (ask_user), still held by the lead: the ticket stays `in_progress` until it times out into a blocked one. */
export const isLeadQuestion = (item: NeedsItem) => !isApproval(item) && item.kind === 'question' && item.status === 'in_progress' && !!item.question?.trim();
/** A blocked ticket that still holds the lead's own claim (timeout-converted question): answered via `superagent.answer` to wake the same lead. */
export const isClaimedBlockedQuestion = (item: NeedsItem): item is NeedsItem & { sessionId: string } =>
  !isApproval(item) && item.kind === 'question' && item.status === 'blocked' && !!item.sessionId && !!item.question?.trim();
/** An owner decision with text: a blocked ticket (today's rule) or a lead's pending question. */
export const isQuestion = (item: NeedsItem) => !isApproval(item) && !!item.question?.trim() && (item.status === 'blocked' || isLeadQuestion(item));
/** A lead without `state` is from a host that predates OIN-80, where every listed lead is working. An idle lead the host unloaded counts as unloaded. */
export const leadState = (lead: Pick<Lead, 'state' | 'idleReason'>): LeadState => (lead.state === 'idle' && lead.idleReason === 'unloaded' ? 'unloaded' : lead.state ?? 'working');

/** Accepts the lead-shaped status and degrades the old worker-shaped one. */
export function normalizeStatus(raw: z.infer<typeof RawStatusSchema>): SuperagentStatus {
  const legacy = raw.leads === undefined;
  const leads: Lead[] = raw.leads ?? (raw.workers ?? []).map((w) => ({
    projectId: w.projectId, sessionId: w.sessionId, presetId: null, model: null, thinking: null, autonomy: w.autonomy, mode: null,
    ticketIds: [w.taskId], step: w.step, tool: w.tool, startedAt: w.startedAt, cost: w.cost, progress: null, legacy: true,
  }));
  const oldReview = raw.needsYou.filter((n) => n.status === 'in_review').map((n) => ({ projectId: n.projectId, taskId: n.taskId, status: n.status }));
  return {
    enabled: raw.enabled, pausedGraphPolicy: raw.pausedGraphPolicy, leads, legacy,
    needsYou: raw.needsYou.filter((n) => n.status !== 'in_review'),
    review: raw.review ?? oldReview,
    projects: raw.projects ?? null,
    notices: raw.notices, leadSettings: readLeadSettings(raw.lead),
  };
}
export const StatusSchema = RawStatusSchema.transform(normalizeStatus);

/** `lead` is sent only when the lead settings are saved: omitting it keeps the host's values. */
export const ConfigSchema = z.object({ enabled: z.boolean(), pausedGraphPolicy: PausedGraphPolicySchema.optional(), lead: LeadSettingsSchema.optional() });
const ConfigReplySchema = z.object({ enabled: z.boolean(), pausedGraphPolicy: PausedGraphPolicySchema.optional(), lead: z.unknown().optional() });
export const RelayTextSchema = z.string().trim().min(1, 'Enter a message to relay.');
export const SuperagentSessionSchema = z.object({ sessionId: z.string(), title: z.string(), createdAt: z.union([z.string(), z.number()]), updatedAt: z.union([z.string(), z.number()]), path: z.string(), project: z.string().nullish(), state: z.object({ kind: z.string() }).nullish(), pendingWork: z.string().nullish() });
export type SuperagentSession = z.infer<typeof SuperagentSessionSchema>;
const SessionsSchema = z.union([z.array(SuperagentSessionSchema), z.object({ sessions: z.array(SuperagentSessionSchema) }).transform((value) => value.sessions)]);

export const PresetSchema = z.object({
  id: z.string().min(1), label: z.string().min(1), description: z.string().default(''),
  model: z.string().nullable().default(null), thinking: z.string().nullable().default(null),
  autonomy: AutonomySchema, mode: z.enum(MODES), maxParallel: z.number().int().nullable().default(null),
  review: z.enum(REVIEWS).default('none'), profiles: z.array(z.string()).nullable().default(null), priority: z.boolean().default(false),
});
export type Preset = z.infer<typeof PresetSchema>;
const PresetListSchema = z.object({ defaultPresetId: z.string().nullish(), presets: z.array(PresetSchema) });

/** `start` refused because the project is not registered / is archived (the host's own code, or the one it carries in `detail`). */
export function startRefusal(error: { code?: unknown; detail?: unknown }): 'not_registered' | 'archived' | null {
  const detail = error.detail && typeof error.detail === 'object' ? (error.detail as { code?: unknown }).code : undefined;
  const codes = [String(error.code), String(detail)];
  if (codes.includes('E_PROJECT_NOT_REGISTERED')) return 'not_registered';
  if (codes.includes('E_PROJECT_ARCHIVED')) return 'archived';
  return null;
}

/** Everything the fleet reports is optional with a safe default: an older host (every node a lead, no groups, no rev) still parses. */
export const FLEET_ROLES = ['superagent', 'lead', 'worker', 'subagent', 'chat'] as const;
export const FLEET_STATUSES = ['working', 'waiting', 'stalled', 'idle', 'ended'] as const;
const nullableString = z.string().nullish().transform((v) => v ?? null);
const nullableMs = z.number().nullish().transform((v) => v ?? null);
/** One agent on the fleet canvas (`superagent.fleet`): the Superagent, a lead, a graph/swarm worker, a subagent or an ordinary chat. */
export const FleetNodeSchema = z.object({
  id: z.string(), parentId: z.string().nullish(), project: nullableString, projectId: nullableString, groupKey: nullableString,
  title: z.string().default(''),
  mode: z.enum(['plain', 'graph', 'swarm']).catch('plain'),
  role: z.enum(FLEET_ROLES).catch('subagent'),
  status: z.enum(FLEET_STATUSES).catch('idle'),
  model: nullableString, name: nullableString, cwd: nullableString,
  ticketIds: z.array(z.string()).catch([]),
  step: nullableString, tool: nullableString,
  question: z.object({ taskId: z.string(), excerpt: z.string().default('') }).nullish().catch(null).transform((v) => v ?? null),
  health: z.object({ error: z.string().default(''), attempts: z.number().default(0), since: nullableMs }).nullish().catch(null).transform((v) => v ?? null),
  lastActiveAt: nullableMs, idleReason: z.enum(['unloaded']).nullish().catch(null).transform((v) => v ?? null),
  startedAt: nullableMs, endedAt: nullableMs,
});
const FleetGroupSchema = z.object({ key: z.string(), name: z.string().default(''), hasBoard: z.boolean().default(false) });
export const FleetSchema = z.object({
  nodes: z.array(FleetNodeSchema).default([]), edges: z.array(z.object({ from: z.string(), to: z.string() })).default([]),
  superagentId: nullableString, groups: z.array(FleetGroupSchema).catch([]), rev: z.string().catch('').default(''),
});
export type FleetNode = z.infer<typeof FleetNodeSchema>;
export type FleetGroupInfo = z.infer<typeof FleetGroupSchema>;
/** `superagentId`, `groups` and `rev` are absent from an older host's payload (and from hand-built test fleets). */
export type Fleet = Omit<z.infer<typeof FleetSchema>, 'superagentId' | 'groups' | 'rev'> & Partial<Pick<z.infer<typeof FleetSchema>, 'superagentId' | 'groups' | 'rev'>>;
/** What an older host answers to an op it does not know: the request enum does not parse (`E_SCHEMA`) or the op is refused (`E_UNSUPPORTED`). */
export const isUnsupportedOp = (code: unknown) => code === 'E_UNSUPPORTED' || code === 'E_SCHEMA' || code === 'E_UNKNOWN_OP';

export const LEAD_ACTIONS = ['retry', 'switch_model', 'stop', 'wake'] as const;
export type LeadAction = (typeof LEAD_ACTIONS)[number];

const RelayReplySchema = z.object({ commentId: z.string().nullish(), sessionId: z.string().nullish(), delivered: z.boolean(), answered: z.string().optional(), started: z.boolean().optional(), saved: z.boolean().optional(), error: z.unknown().optional() });
const AnswerReplySchema = z.object({ sessionId: z.string().nullish(), delivered: z.boolean(), answered: z.string().optional(), started: z.boolean().optional(), saved: z.boolean().optional(), error: z.unknown().optional() });

export type RelayDelivery = 'queue' | 'steer';
export type StartLead = 'auto' | 'reuse' | 'new';
export const superagentApi = {
  fleet: (host: string) => hostCall(host, 'superagent.fleet', {}, FleetSchema as unknown as z.ZodType<Fleet>),
  /** Lead health actions. An older host answers "unknown method"; the caller shows that error, it is never a success. */
  leadAction: (host: string, sessionId: string, action: LeadAction, model?: string) => hostCall(host, 'superagent.lead_action', { sessionId, action, ...(model ? { model } : {}) }, z.object({ ok: z.literal(true) })),
  sessions: (host: string) => hostCall(host, 'superagent.sessions', {}, SessionsSchema),
  status: (host: string) => hostCall(host, 'superagent.status', {}, StatusSchema),
  configure: (host: string, config: z.infer<typeof ConfigSchema>) => hostCall(host, 'superagent.configure', ConfigSchema.parse(config), ConfigReplySchema),
  configureProject: (host: string, projectId: string, autoStart: boolean) => hostCall(host, 'superagent.configure_project', { projectId, autoStart }, z.object({ projectId: z.string(), autoStart: z.boolean() })),
  /** The project's lead model + thinking (both null clears; a running lead keeps its model until restarted). */
  configureProjectLead: (host: string, projectId: string, model: string | null, thinking: string | null) => hostCall(host, 'superagent.configure_project_lead', { projectId, model, thinking }, z.object({ projectId: z.string(), model: z.string().nullish(), thinking: z.string().nullish() })),
  setMode: (host: string, projectId: string, mode: LeadMode) => hostCall(host, 'superagent.set_mode', { projectId, mode }, z.object({ sessionId: z.string().nullish(), mode: z.string() })),
  /** `lead` (OIN-80): `auto` (default) reuses a live or unloaded lead of the project, `new` always opens a new one. */
  start: (host: string, projectId: string, taskIds: string[], presetId?: string | null, lead?: StartLead) => hostCall(host, 'superagent.start', { projectId, taskIds, ...(presetId ? { presetId } : {}), ...(lead ? { lead } : {}) }, z.object({ sessionId: z.string() })),
  /** Clears one blocker: register a workspace, unarchive, or switch Auto on. `register` needs `path` and returns the new slug. */
  enable: (host: string, input: { action: EnableAction; projectId?: string; path?: string }) => hostCall(host, 'superagent.enable', {
    action: input.action, ...(input.projectId ? { projectId: input.projectId } : {}), ...(input.path ? { path: input.path } : {}),
  }, z.object({ ok: z.literal(true), projectId: z.string().nullish() })),
  stop: (host: string, projectId: string, taskId: string) => hostCall(host, 'superagent.stop', { projectId, taskId }, z.object({ stopped: z.boolean() })),
  approvalResponse: (host: string, projectId: string, taskId: string, workerSessionId: string, requestId: string, decision: 'allow_once' | 'always' | 'deny') => hostCall(host, 'superagent.approval_response', { projectId, taskId, workerSessionId, requestId, decision }, z.object({ accepted: z.literal(true) })),
  relay: (host: string, input: { projectId: string; text: string; delivery: RelayDelivery; taskId?: string; taskIds?: string[]; priority?: boolean; sessionId?: string; requestId?: string }) => hostCall(host, 'superagent.relay', { projectId: input.projectId, text: RelayTextSchema.parse(input.text), delivery: input.delivery, ...(input.taskId ? { taskId: input.taskId } : {}), ...(input.taskIds ? { taskIds: input.taskIds } : {}), ...(input.sessionId ? { sessionId: input.sessionId } : {}), ...(input.requestId ? { requestId: input.requestId } : {}), priority: input.priority ?? false }, RelayReplySchema),
  /** Answers a blocked ticket holding the lead's own claim (`superagent.answer`), waking the same lead. Older hosts answer E_SCHEMA/E_UNSUPPORTED. */
  answer: (host: string, sessionId: string, taskId: string, answer: string) => hostCall(host, 'superagent.answer', { sessionId, taskId, answer: RelayTextSchema.parse(answer) }, AnswerReplySchema),
  presets: {
    list: (host: string) => hostCall(host, 'superagent.presets.list', {}, PresetListSchema),
    save: (host: string, preset: Preset) => hostCall(host, 'superagent.presets.save', { preset: PresetSchema.parse(preset) }, z.object({ preset: PresetSchema })),
    remove: (host: string, presetId: string) => hostCall(host, 'superagent.presets.delete', { presetId }, z.object({ deleted: z.boolean() })),
    setDefault: (host: string, presetId: string) => hostCall(host, 'superagent.presets.set_default', { presetId }, z.object({ defaultPresetId: z.string() })),
  },
};

/** Inspect a history session without treating metadata-only entries as messages. */
export function isEmptySuperagentSession(hostId: string, id: string, client: HostClient = hostClient(hostId)): Promise<boolean> {
  return new Promise((resolve) => {
    let done = false;
    let stream: ReturnType<HostClient['openSession']> | undefined;
    const finish = (empty: boolean) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      stream?.close();
      resolve(empty);
    };
    const timer = setTimeout(() => finish(false), 3000);
    stream = client.openSession({ kind: 'open', session: id }, {
      snapshot(snapshot) {
        const state = snapshot.state as { isStreaming?: boolean } | undefined;
        const empty = snapshot.reset && !snapshot.page?.has_more && !snapshot.waking && !state?.isStreaming && !!snapshot.entries && !snapshot.entries.some((entry) => (entry as { type?: string }).type === 'message');
        finish(empty);
      },
      events() {},
      closed() { finish(false); },
    });
    if (done) stream.close();
  });
}

/** Creation is a session stream, never an ordinary session_new or a shell command. */
export function openSuperagentSession(hostId: string, client: HostClient = hostClient(hostId)): Promise<string> {
  return new Promise((resolve, reject) => {
    let closed = false;
    const timer = setTimeout(() => { closed = true; stream.close(); reject(new Error('The host did not open Superagent. Retry.')); }, 20_000);
    const stream = client.openSession({ kind: 'superagent' }, {
      snapshot(snapshot) {
        clearTimeout(timer);
        if (closed) return;
        closed = true;
        // The host materializes the restricted session before answering. Keep the
        // creating stream alive briefly while Conversation attaches by exact id.
        setTimeout(() => stream.close(), 6000);
        resolve(snapshot.session_id);
      },
      events() {},
      closed(error) {
        clearTimeout(timer);
        if (closed) return;
        closed = true;
        reject(toAppError(error ?? new Error('The host closed Superagent. Retry.')));
      },
    });
  });
}
