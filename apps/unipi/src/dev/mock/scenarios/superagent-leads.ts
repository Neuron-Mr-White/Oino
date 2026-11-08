/**
 * Superagent on the lead model (v3 r5): two live leads (a swarm and a graph),
 * three parked approvals, two blocked questions, three review tickets and five
 * projects with mixed auto-start. Ops are stateful so toggles, mode switches,
 * starts and presets behave in a capture. `?scenario=superagent-leads`.
 */
import type { MockScenario } from '../host';
import { genSession } from '../gen';
import { superagent } from './superagent';

const base = superagent.hosts![0]!;
const chat = structuredClone(base.sessions!['superagent-chat']!);
chat.snapshot.entries = [
  { type: 'custom', id: 'sa-kind', parentId: null, customType: 'oino:superagent', data: { workspace: null } },
  { type: 'message', id: 'sa-user', parentId: 'sa-kind', message: { role: 'user', content: 'Fix the sync bugs in Heartpick and ship the Oino graph work.' } },
  { type: 'message', id: 'sa-answer', parentId: 'sa-user', message: { role: 'assistant', content: [{ type: 'text', text: 'Both projects have a lead running.\n\n- **Heartpick** runs a bug-batch swarm on three tickets.\n- **Oino** runs a feature graph.\n\nThree approvals and two questions need you.' }], stopReason: 'stop' } },
] as never;
chat.snapshot.leaf_id = 'sa-answer';

const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();
type MockProject = { projectId: string; name: string; autoStart: boolean; ready: number; lead: string | null; registered?: boolean; archived?: boolean; path?: string };
const projects: MockProject[] = [
  { projectId: 'heartpick', name: 'Heartpick', autoStart: true, ready: 0, lead: 'lead-hp' },
  { projectId: 'oino', name: 'Oino', autoStart: false, ready: 0, lead: 'lead-oino' },
  { projectId: 'website', name: 'Website', autoStart: true, ready: 3, lead: null },
  { projectId: 'orchard', name: 'Orchard', autoStart: false, ready: 2, lead: null },
  { projectId: 'billing', name: 'Billing', autoStart: false, ready: 0, lead: null },
  { projectId: 'notes', name: 'Field notes', autoStart: false, ready: 0, lead: null },
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
const leads = [
  { projectId: 'heartpick', sessionId: 'lead-hp', presetId: 'bug-batch', model: 'anthropic/claude-sonnet-4-5', thinking: 'high', autonomy: 'high', mode: 'swarm', ticketIds: ['HP-1', 'HP-4', 'HP-7'], step: 'Fixing the sync race', tool: 'edit', startedAt: ago(18), cost: 0.84, progress: { running: 2, queued: 1, done: 3 } },
  { projectId: 'oino', sessionId: 'lead-oino', presetId: 'feature', model: 'openai/gpt-5', thinking: 'medium', autonomy: 'high', mode: 'graph', ticketIds: ['OIN-14', 'OIN-16', 'OIN-18', 'OIN-19'], step: 'Running the workspace tests', tool: 'bash', startedAt: ago(47), cost: 2.31, progress: { running: 1, queued: 2, done: 0 } },
];
const approval = (projectId: string, taskId: string, workerSessionId: string, requestId: string, command: string, summary: string, risk: string) => ({ projectId, taskId, status: 'blocked', workerSessionId, requestId, sessionId: workerSessionId, tool: 'bash', command, summary, risk, ceiling: 'medium', reason: 'needs Autonomy High' });
const needsYou = [
  approval('heartpick', 'HP-1', 'lead-hp', 'req-1', 'git push origin fix/sync-race', 'Push the sync fix branch', 'high'),
  approval('oino', 'OIN-14', 'lead-oino', 'req-2', 'cargo publish --dry-run -p oino-core', 'Check the crate packages cleanly', 'high'),
  approval('heartpick', 'HP-4', 'lead-hp', 'req-3', 'rm -rf target/debug/incremental', 'Clear the stale incremental cache', 'high'),
  { projectId: 'website', taskId: 'WEB-8', status: 'blocked', question: 'Which domain should the staging deploy use: staging.example.com or preview.example.com?' },
  { projectId: 'billing', taskId: 'BIL-3', status: 'blocked', question: 'The invoice export needs a currency. Should totals be EUR or USD?' },
];
const review = [{ projectId: 'heartpick', taskId: 'HP-3', status: 'in_review' }, { projectId: 'oino', taskId: 'OIN-12', status: 'in_review' }, { projectId: 'notes', taskId: 'NOT-2', status: 'in_review' }];
const titles: Record<string, string> = {
  'HP-1': 'Sync drops edits after reconnect', 'HP-3': 'Retry banner flickers', 'HP-4': 'Cache never invalidates', 'HP-7': 'Duplicate rows on import',
  'OIN-12': 'Autonomy controls and approvals', 'OIN-14': 'Superagent interface', 'OIN-16': 'Lead presets', 'OIN-18': 'Mode switch gate', 'OIN-19': 'Phone sheet',
  'WEB-8': 'Staging deployment', 'WEB-9': 'Pricing table copy', 'WEB-11': 'Add a changelog page', 'BIL-3': 'Invoice CSV export', 'NOT-2': 'Tag filter in the sidebar',
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
// `?saCapture=globalOff` opens with the master Auto-start switch off.
let enabled = !(typeof location !== 'undefined' && new URLSearchParams(location.search).get('saCapture') === 'globalOff');
let concurrencyCap = 2;

export const superagentLeads: MockScenario = {
  name: 'superagent-leads',
  hosts: [{
    ...base,
    cards: [{ session_id: 'lead-hp', name: 'Lead · Heartpick', cwd: '/home/you/projects/heartpick', run_state: 'working', updated_at: Date.now(), message_count: 4, worker: { projectId: 'heartpick', projectName: 'Heartpick', taskId: 'HP-1' } }],
    sessions: { 'superagent-chat': chat, 'lead-hp': genSession('lead-hp', { seed: 5, turns: 1, toolsPerTurn: 1, cwd: '/home/you/projects/heartpick' }), 'lead-oino': genSession('lead-oino', { seed: 6, turns: 1, toolsPerTurn: 1, cwd: '/home/you/projects/oino' }) },
    ops: {
      ...base.ops,
      'superagent.status': () => ({ enabled, concurrencyCap, leads, needsYou, review, projects: rows() }),
      'superagent.configure': (a) => { enabled = a.enabled as boolean; concurrencyCap = a.concurrencyCap as number; return { enabled, concurrencyCap }; },
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
      'superagent.relay': () => ({ commentId: null, sessionId: 'lead-hp', delivered: true }),
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
