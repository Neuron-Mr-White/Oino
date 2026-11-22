/**
 * Superagent on the lead model (v3 r5): two live leads (a swarm and a graph),
 * three parked approvals, two blocked questions, three review tickets and five
 * projects with mixed auto-start. Ops are stateful so toggles, mode switches,
 * starts and presets behave in a capture. `?scenario=superagent-leads`.
 *
 * OIN-80/81/82 captures: `&saCapture=states` adds a waiting, a stalled, an idle and an unloaded lead, a lead's own
 * question in progress (WEB-9, with its countdown) and a "Board created for Orchard" notice. Flags combine with commas:
 *   `leadUnsupported`  `superagent.lead_action` answers "unknown method" (this host can't do that yet)
 *   `leadState`        every lead action is refused with E_LEAD_STATE
 *   `unknownModel`     switch_model is refused with E_UNKNOWN_MODEL
 *   `answered`         relaying the lead's question answers E_ALREADY_ANSWERED
 * The lead settings (`superagent.configure` with `lead`) are kept and reported back by `superagent.status`.
 */
import type { MockScenario } from '../host';
import { genSession } from '../gen';
import { superagent } from './superagent';
import { CLO6_REPORT } from '../../../components/superagent/clo6.fixture';

const base = superagent.hosts![0]!;
const chat = structuredClone(base.sessions!['superagent-chat']!);
chat.snapshot.entries = [
  { type: 'custom', id: 'sa-kind', parentId: null, customType: 'oino:superagent', data: { workspace: null } },
  { type: 'message', id: 'sa-user', parentId: 'sa-kind', message: { role: 'user', content: 'Fix the sync bugs in ExampleApp and ship the Oino graph work.' } },
  { type: 'message', id: 'sa-answer', parentId: 'sa-user', message: { role: 'assistant', content: [{ type: 'text', text: 'Both projects have a lead running.\n\n- **ExampleApp** runs a bug-batch swarm on three tickets.\n- **Oino** runs a feature graph.\n\nThree approvals and two questions need you.' }], stopReason: 'stop' } },
] as never;
chat.snapshot.leaf_id = 'sa-answer';

// `?saCapture=clo6` puts the long CLO-6 build report (question at the very end) first in the queue (OIN-62).
const clo6 = typeof location !== 'undefined' && new URLSearchParams(location.search).get('saCapture') === 'clo6';
const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();
const msAgo = (min: number) => Date.now() - min * 60_000;
const captureParams = typeof location === 'undefined' ? new URLSearchParams() : new URLSearchParams(location.hash.includes('?') ? location.hash.slice(location.hash.indexOf('?') + 1) : location.search);
const caps = new Set((captureParams.get('saCapture') ?? new URLSearchParams(typeof location === 'undefined' ? '' : location.search).get('saCapture') ?? '').split(',').filter(Boolean));
const fail = (code: string, message: string) => ({ code, message, retryable: false });
type MockProject = { projectId: string; name: string; autoStart: boolean; ready: number; lead: string | null; registered?: boolean; archived?: boolean; path?: string };
const projects: MockProject[] = [
  { projectId: 'example-app', name: 'ExampleApp', autoStart: true, ready: 0, lead: 'lead-hp' },
  { projectId: 'oino', name: 'Oino', autoStart: false, ready: 0, lead: 'lead-oino' },
  { projectId: 'website', name: 'Website', autoStart: true, ready: 3, lead: null },
  { projectId: 'orchard', name: 'Orchard', autoStart: false, ready: 2, lead: null },
  { projectId: 'billing', name: 'Billing', autoStart: false, ready: 0, lead: null },
  { projectId: 'notes', name: 'Field notes', autoStart: false, ready: 0, lead: null },
  ...(clo6 ? [{ projectId: 'cloud', name: 'Cloud', autoStart: false, ready: 0, lead: null }] : []),
  // A recent workspace with no Kanboard board: the host suggests an id and a path.
  { projectId: 'side-quest', name: 'side-quest', autoStart: false, ready: 0, lead: null, registered: false, path: '/home/you/Projects/side-quest' },
];
/** Why a project cannot take work, the way the host reports it (OIN-27). */
function blockersOf(p: MockProject) {
  const out: { code: string; message: string; action: { kind: string; label: string } }[] = [];
  if (p.registered === false) out.push({ code: 'not_registered', message: `${p.name} has no Kanboard board, so Superagent has no tickets to work.`, action: { kind: 'register', label: 'Set up Kanboard' } });
  else if (p.archived) out.push({ code: 'archived', message: `${p.name} is archived, so Superagent will not start work in it.`, action: { kind: 'unarchive', label: 'Unarchive' } });
  else {
    if (!p.autoStart) out.push({ code: 'project_auto_off', message: `Auto is off for ${p.name}, so ready tickets wait for a manual start.`, action: { kind: 'enable_project_auto', label: 'Turn on Auto' } });
    if (!enabled) out.push({ code: 'global_auto_off', message: 'Auto-start is off for every project, so ready tickets wait for a manual start.', action: { kind: 'enable_auto_start', label: 'Turn on Auto-start' } });
  }
  return out;
}
const rows = () => projects.map((p) => ({ ...p, blockers: blockersOf(p), registered: p.registered !== false }));
/** The host refuses a start the same way: E_BOARD_REJECTED carrying the project code in `detail`. */
const refuse = (code: string, message: string) => ({ code: 'E_BOARD_REJECTED', message, retryable: false, detail: { code } });
const leads: Array<Record<string, unknown> & { sessionId: string; projectId: string }> = [
  { projectId: 'example-app', sessionId: 'lead-hp', presetId: 'bug-batch', model: 'anthropic/claude-sonnet-4-5', thinking: 'high', autonomy: 'high', mode: 'swarm', ticketIds: ['HP-1', 'HP-4', 'HP-7'], step: 'Fixing the sync race', tool: 'edit', startedAt: ago(18), cost: 0.84, progress: { running: 2, queued: 1, done: 3 } },
  { projectId: 'oino', sessionId: 'lead-oino', presetId: 'feature', model: 'openai/gpt-5', thinking: 'medium', autonomy: 'high', mode: 'graph', ticketIds: ['OIN-14', 'OIN-16', 'OIN-18', 'OIN-19'], step: 'Running the workspace tests', tool: 'bash', startedAt: ago(47), cost: 2.31, progress: { running: 1, queued: 2, done: 0 } },
];
if (caps.has('states')) {
  const lead = (projectId: string, sessionId: string, over: Record<string, unknown>) => ({ projectId, sessionId, presetId: 'feature', model: 'anthropic/claude-sonnet-4-5', thinking: 'medium', autonomy: 'high', mode: 'none', ticketIds: [], startedAt: ago(95), cost: 0.4, progress: null, ...over });
  leads.push(
    lead('website', 'lead-web', { state: 'waiting', ticketIds: ['WEB-9'], step: 'Waiting for your answer', question: { taskId: 'WEB-9', excerpt: 'Which currency should the pricing table show first: EUR or USD?' }, lastActiveAt: msAgo(4) }),
    lead('billing', 'lead-bill', { state: 'stalled', ticketIds: ['BIL-3'], model: 'anthropic/claude-opus-4-1', health: { kind: 'model', error: '429: quota exhausted, resets after 79h', attempts: 8, since: msAgo(14), nextProbeAt: Date.now() + 31 * 60_000 }, lastActiveAt: msAgo(14) }),
    lead('orchard', 'lead-orchard', { state: 'idle', lastActiveAt: msAgo(12) }),
    lead('notes', 'lead-notes', { state: 'unloaded', lastActiveAt: msAgo(52) }),
  );
  for (const [id, session] of [['website', 'lead-web'], ['billing', 'lead-bill'], ['orchard', 'lead-orchard'], ['notes', 'lead-notes']] as const) { const p = projects.find((x) => x.projectId === id); if (p) p.lead = session; }
}
const approval = (projectId: string, taskId: string, workerSessionId: string, requestId: string, command: string, summary: string, risk: string) => ({ kind: 'approval', projectId, taskId, status: 'blocked', workerSessionId, requestId, sessionId: workerSessionId, tool: 'bash', command, summary, risk, ceiling: 'medium', reason: 'needs Autonomy High' });
const needsYou = [
  ...(clo6 ? [{ kind: 'question', projectId: 'cloud', taskId: 'CLO-6', status: 'blocked', question: CLO6_REPORT }] : []),
  approval('example-app', 'HP-1', 'lead-hp', 'req-1', 'git push origin fix/sync-race', 'Push the sync fix branch', 'high'),
  approval('oino', 'OIN-14', 'lead-oino', 'req-2', 'cargo publish --dry-run -p oino-core', 'Check the crate packages cleanly', 'high'),
  approval('example-app', 'HP-4', 'lead-hp', 'req-3', 'rm -rf target/debug/incremental', 'Clear the stale incremental cache', 'high'),
  ...(caps.has('states') ? [{ kind: 'question', projectId: 'website', taskId: 'WEB-9', status: 'in_progress', question: 'Which currency should the pricing table show first: EUR or USD?', sessionId: 'lead-web', requestId: 'req-web-9', options: [['EUR', 'USD']], askedAt: msAgo(4), expiresAt: Date.now() + 26 * 60_000 }] : []),
  { kind: 'question', projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain should the staging deploy use: staging.example.com or preview.example.com?' },
  { kind: 'question', projectId: 'billing', taskId: 'BIL-3', status: 'blocked', sessionId: 'lead-bill', question: 'The invoice export needs a currency. Should totals be EUR or USD?' },
];
const review = [{ projectId: 'example-app', taskId: 'HP-3', status: 'in_review' }, { projectId: 'oino', taskId: 'OIN-12', status: 'in_review' }, { projectId: 'notes', taskId: 'NOT-2', status: 'in_review' }];
const titles: Record<string, string> = {
  'HP-1': 'Sync drops edits after reconnect', 'HP-3': 'Retry banner flickers', 'HP-4': 'Cache never invalidates', 'HP-7': 'Duplicate rows on import',
  'OIN-12': 'Autonomy controls and approvals', 'OIN-14': 'Superagent interface', 'OIN-16': 'Lead presets', 'OIN-18': 'Mode switch gate', 'OIN-19': 'Phone sheet',
  'WEB-8': 'Staging deployment', 'WEB-9': 'Pricing table copy', 'WEB-11': 'Add a changelog page', 'BIL-3': 'Invoice CSV export', 'CLO-6': 'Invoice service v2', 'NOT-2': 'Tag filter in the sidebar',
};
const ready = [{ id: 'WEB-9', priority: 'high' }, { id: 'WEB-11', priority: 'medium' }, { id: 'WEB-12', priority: 'low' }];
let presets = [
  { id: 'quick-fix', label: 'Quick fix', description: 'One small change, no review', model: null, thinking: null, autonomy: 'high', mode: 'none', maxParallel: null, review: 'none', profiles: null, priority: false },
  { id: 'bug-batch', label: 'Bug batch', description: 'Batch independent bug fixes', model: null, thinking: null, autonomy: 'high', mode: 'swarm', maxParallel: 4, review: 'light', profiles: null, priority: false },
  { id: 'feature', label: 'Feature', description: 'Plan, build and review', model: null, thinking: null, autonomy: 'high', mode: 'graph', maxParallel: 3, review: 'full', profiles: null, priority: false },
  { id: 'urgent', label: 'Urgent', description: 'Jump the queue', model: null, thinking: null, autonomy: 'high', mode: 'none', maxParallel: null, review: 'none', profiles: null, priority: true },
  { id: 'research', label: 'Research', description: 'Read and report', model: null, thinking: null, autonomy: 'medium', mode: 'none', maxParallel: null, review: 'none', profiles: null, priority: false },
];
let defaultPresetId = 'quick-fix';
let leadConfig = { retries: 3, fallbackModel: null as string | null, warmMinutes: 30, questionTimeoutMinutes: 30 };
const notices = caps.has('states') ? [{ kind: 'board_created', projectId: 'orchard', name: 'Orchard', at: msAgo(1), text: 'Board created for Orchard (orchard).' }] : [];
/** Lead health actions: retry/wake/switch_model bring the lead back to work, stop ends it. */
function leadAction(a: Record<string, unknown>) {
  if (caps.has('leadUnsupported')) throw fail('E_SCHEMA', 'unknown variant `superagent.lead_action`');
  if (caps.has('leadState')) throw fail('E_LEAD_STATE', 'The lead is working again; nothing to retry.');
  const at = leads.findIndex((l) => l.sessionId === a.sessionId);
  if (at < 0) throw fail('E_LEAD_UNKNOWN', `No lead ${String(a.sessionId)}`);
  if (a.action === 'stop') { leads.splice(at, 1); return { ok: true }; }
  if (a.action === 'switch_model') {
    if (caps.has('unknownModel')) throw fail('E_UNKNOWN_MODEL', `Unknown model ${String(a.model)}`);
    leads[at]!.model = a.model;
  }
  Object.assign(leads[at]!, { state: 'working', health: null, idleReason: null, step: a.action === 'wake' ? 'Waking up' : 'Retrying' });
  return { ok: true };
}
// `?saCapture=globalOff` opens with the master Auto-start switch off.
let enabled = !(typeof location !== 'undefined' && new URLSearchParams(location.search).get('saCapture') === 'globalOff');

export const superagentLeads: MockScenario = {
  name: 'superagent-leads',
  hosts: [{
    ...base,
    cards: [{ session_id: 'lead-hp', name: 'Lead · ExampleApp', cwd: '/home/you/projects/example-app', run_state: 'working', updated_at: Date.now(), message_count: 4, worker: { projectId: 'example-app', projectName: 'ExampleApp', taskId: 'HP-1' } }],
    sessions: { 'superagent-chat': chat, 'lead-hp': genSession('lead-hp', { seed: 5, turns: 1, toolsPerTurn: 1, cwd: '/home/you/projects/example-app' }), 'lead-oino': genSession('lead-oino', { seed: 6, turns: 1, toolsPerTurn: 1, cwd: '/home/you/projects/oino' }) },
    ops: {
      ...base.ops,
      'superagent.status': () => ({ enabled, leads, needsYou, review, projects: rows(), lead: leadConfig, ...(notices.length ? { notices } : {}) }),
      'superagent.configure': (a) => { enabled = a.enabled as boolean; if (a.lead) leadConfig = { ...leadConfig, ...(a.lead as typeof leadConfig) }; return { enabled, lead: leadConfig }; },
      'superagent.lead_action': leadAction,
      'superagent.configure_project': (a) => { const p = projects.find((x) => x.projectId === a.projectId); if (p) p.autoStart = a.autoStart as boolean; return { projectId: a.projectId, autoStart: a.autoStart }; },
      'superagent.set_mode': (a) => { const l = leads.find((x) => x.projectId === a.projectId); if (l) l.mode = a.mode as string; return { sessionId: l?.sessionId ?? null, mode: a.mode }; },
      'superagent.start': (a) => {
        const p = projects.find((x) => x.projectId === a.projectId);
        if (p?.registered === false) throw refuse('E_PROJECT_NOT_REGISTERED', blockersOf(p)[0]!.message);
        if (p?.archived) throw refuse('E_PROJECT_ARCHIVED', blockersOf(p)[0]!.message);
        return { sessionId: 'lead-hp' };
      },
      'superagent.enable': (a) => {
        const p = projects.find((x) => x.projectId === a.projectId || (a.path !== undefined && x.path === a.path));
        switch (a.action) {
          case 'register': if (!p) throw refuse('E_PROJECT_NOT_REGISTERED', 'Unknown workspace.'); p.registered = true; return { ok: true, projectId: p.projectId };
          case 'unarchive': if (p) p.archived = false; return { ok: true, projectId: p?.projectId ?? null };
          case 'enable_project_auto': if (p) p.autoStart = true; return { ok: true, projectId: p?.projectId ?? null };
          default: enabled = true; return { ok: true, projectId: null };
        }
      },
      'superagent.stop': () => ({ stopped: true }),
      'superagent.relay': (a) => {
        const targetTask = (a.taskId as string | undefined) ?? (a.taskIds as string[] | undefined)?.[0];
        const asked = needsYou.findIndex((n) => n.status === 'in_progress' && n.taskId === targetTask);
        if (asked >= 0) {
          if (caps.has('answered')) throw fail('E_ALREADY_ANSWERED', 'Question already answered');
          needsYou.splice(asked, 1);
          return { sessionId: 'lead-web', delivered: true, answered: 'question' };
        }
        return { commentId: null, sessionId: 'lead-hp', delivered: true };
      },
      'superagent.answer': (a) => {
        if (caps.has('leadUnsupported')) throw fail('E_SCHEMA', 'unknown variant `superagent.answer`');
        if (caps.has('answered')) throw fail('E_ALREADY_ANSWERED', 'This question was already answered.');
        const idx = needsYou.findIndex((n) => n.status === 'blocked' && n.taskId === a.taskId && (n as { sessionId?: string }).sessionId === a.sessionId);
        if (idx >= 0) needsYou.splice(idx, 1);
        return { sessionId: a.sessionId as string, delivered: true, answered: 'blocked' };
      },
      'superagent.presets.list': () => ({ defaultPresetId, presets }),
      'superagent.presets.save': (a) => { const p = a.preset as (typeof presets)[number]; presets = presets.some((x) => x.id === p.id) ? presets.map((x) => (x.id === p.id ? p : x)) : [...presets, p]; return { preset: p }; },
      'superagent.presets.delete': (a) => { presets = presets.filter((x) => x.id !== a.presetId); return { deleted: true }; },
      'superagent.presets.set_default': (a) => { defaultPresetId = a.presetId as string; return { defaultPresetId }; },
      board_cli: (args) => {
        const [cmd, id] = args.args as string[];
        if (cmd === 'list') return { ok: true, payload: { problems: [], tasks: ready.map((r, i) => ({ id: r.id, title: titles[r.id] ?? `Ticket ${r.id}`, status: 'todo', priority: r.priority, order: i, deps: [], labels: [], created: '', updated: '', activity: [], ready: true })) } };
        return { ok: true, payload: { id, title: titles[id!] ?? id, autonomy: 'high', status: 'blocked', priority: 'medium', order: 0, deps: [], labels: [], created: '', updated: '', activity: [] } };
      },
    },
  }],
  setup() { location.hash = '#/superagent?session=superagent-chat'; },
};
