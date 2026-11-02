/**
 * Model / thinking / working-mode scenarios (WP-A14):
 *
 *   model-picker   the model popover: Now line, shared search, Single rows with
 *                  Vision + context badges, current checked (Z-69)
 *   model-fusion   the popover on its Fusion view: unipi.fusion on, a running
 *                  pair, Lead/Sidekick columns filtered by the one search,
 *                  effort pills (Z-76)
 *   thinking-chip  the composer's model / thinking / context controls and the
 *                  thinking menu (Z-71, Z-73)
 *   mode-chip      the header mode chip over a running Graph, its switch menu
 *                  and a starter already in the composer (Z-74)
 */
import type { MockHostSpec, MockScenario, MockSession } from '../host';
import { genCards, genSession } from '../gen';
import { openModelPicker } from '../../../components/model/target';
import { setComposerText } from '../../../lib/composer/drafts';

const HOST = 'this-pc';
const CWD = '/home/you/projects/unipi';
const SID = 's001';

const topic = (name: string, data: unknown) => ({ topic: name, data, sticky: true, publisher: 'unipi.long-horizon' });

const LH_EXTENSION = { id: 'unipi.long-horizon', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/lh.wasm' };
const FUSION_EXTENSION = { id: 'unipi.fusion', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/fusion.wasm' };
const WORKFLOW_EXTENSION = { id: 'unipi.workflow', kind: 'extension', enabled: true, loaded: true, path: '/opt/unipi/ext/workflow.wasm' };

/** The context numbers the ring and the Session modal read. */
const CONTEXT_CMD = { '_oino/get_context_usage': () => ({ tokens: 74_120, contextWindow: 200_000, percent: 37.1 }) };

const MODE_COMMANDS = [
  { name: 'goal', description: 'Goal mode' },
  { name: 'ralph', description: 'Ralph loop' },
  { name: 'graph', description: 'Graph mode' },
  { name: 'swarm', description: 'Swarm mode' },
  { name: 'regular', description: 'Plain mode' },
];

type SessionOpts = {
  model?: { provider: string; id: string; name: string };
  thinking?: string;
  commands?: unknown[];
  topics?: unknown[];
  streaming?: boolean;
  cmd?: MockSession['cmd'];
};

function makeSession(opts: SessionOpts = {}): MockSession {
  const base = genSession(SID, { seed: 11, turns: 3, toolsPerTurn: 1, cwd: CWD });
  const model = opts.model ?? { provider: 'anthropic', id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5' };
  return {
    ...base,
    ...(opts.cmd ? { cmd: opts.cmd } : {}),
    snapshot: {
      ...base.snapshot,
      state: { ...(base.snapshot.state as object), model, thinkingLevel: opts.thinking ?? 'high', ...(opts.streaming ? { isStreaming: true } : {}) },
      ...(opts.commands ? { commands: opts.commands } : {}),
      ...(opts.topics ? { topics: opts.topics } : {}),
    },
  };
}

function host(s: MockSession): MockHostSpec {
  return {
    id: HOST,
    name: 'workstation',
    transport: 'local',
    conn: { state: 'connected' },
    cards: genCards(4, { seed: 5, changes: true }).map((c) => ({ ...c, cwd: CWD })),
    sessions: { [SID]: s },
  };
}

/** Opens the shown session on both shells (`/` reopens the last one; the
 *  phone's `/` would go to its Chat list, so the route is written directly). */
function seedRoute(): void {
  localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: SID }));
  location.hash = `#/s/${HOST}/${SID}`;
}

const seedPicker = () => {
  seedRoute();
  openModelPicker({ hostId: HOST, sessionId: SID });
};

/** `/scoped-models`: the picker with the Cycle set block open (WP-A25, G-15). */
const seedCyclePicker = () => {
  seedRoute();
  openModelPicker({ hostId: HOST, sessionId: SID, focus: 'cycle' });
};

export const modelPicker: MockScenario = {
  name: 'model-picker',
  hosts: [host(makeSession({ cmd: { ...CONTEXT_CMD, '_oino/extensions': () => ({ extensions: [WORKFLOW_EXTENSION] }) } }))],
  setup: seedPicker,
};

export const modelCycleSet: MockScenario = {
  name: 'model-cycle-set',
  hosts: [
    {
      ...host(makeSession({ cmd: { ...CONTEXT_CMD, '_oino/extensions': () => ({ extensions: [WORKFLOW_EXTENSION] }) } })),
      settings: { pi: { enabledModels: ['anthropic/claude-sonnet-4-5', 'anthropic/*'] } },
    },
  ],
  setup: seedCyclePicker,
};

export const modelFusion: MockScenario = {
  name: 'model-fusion',
  hosts: [
    host(
      makeSession({
        model: { provider: 'anthropic', id: 'claude-opus-4-1', name: 'Claude Opus 4.1' },
        thinking: 'high',
        cmd: {
          ...CONTEXT_CMD,
          '_oino/extensions': () => ({ extensions: [WORKFLOW_EXTENSION, FUSION_EXTENSION] }),
          '_oino/service_call': (cmd) => {
            if (cmd.name === 'fusion.status') {
              return {
                active: { kind: 'fusion', lead: 'anthropic/claude-opus-4-1', sidekick: 'zai/glm-4.6', leadEffort: 'high', sidekickEffort: 'low' },
                status: {
                  leadName: 'Claude Opus 4.1',
                  leadKey: 'anthropic/claude-opus-4-1',
                  sidekickName: 'GLM-4.6',
                  sidekickKey: 'zai/glm-4.6',
                  leadEffort: 'high',
                  sidekickEffort: 'low',
                  savedUsd: 2.14,
                },
              };
            }
            if (cmd.name === 'fusion.set-pair') return { ok: true };
            throw Object.assign(new Error(`Unknown service: ${cmd.name}`), { code: 'E_NOT_FOUND' });
          },
        },
      }),
    ),
  ],
  setup: seedPicker,
};

export const thinkingChip: MockScenario = {
  name: 'thinking-chip',
  hosts: [
    host(
      makeSession({
        commands: MODE_COMMANDS,
        cmd: { ...CONTEXT_CMD, '_oino/extensions': () => ({ extensions: [WORKFLOW_EXTENSION] }) },
      }),
    ),
  ],
  setup: seedRoute,
};

const GRAPH_COUNTS = { queued: 1, running: 2, blocked: 0, completed: 3, failed: 0, stopped: 0, cancelled: 0, superseded: 0 };

export const modeChip: MockScenario = {
  name: 'mode-chip',
  hosts: [
    host(
      makeSession({
        commands: MODE_COMMANDS,
        streaming: true,
        topics: [
          topic('unipi:long-horizon:state', { mode: 'graph', owner: { kind: 'graph', status: 'active' } }),
          topic('unipi:long-horizon:progress', {
            mode: 'graph',
            graph: { graphId: 'g-77', mode: 'graph', task: 'Land the model, thinking and mode controls (WP-A14)', counts: GRAPH_COUNTS },
          }),
        ],
        cmd: { ...CONTEXT_CMD, '_oino/extensions': () => ({ extensions: [LH_EXTENSION, WORKFLOW_EXTENSION] }), prompt: () => ({}) },
      }),
    ),
  ],
  setup: () => {
    seedRoute();
    // A starter already in the composer (the "starter" shot shows it prefilled).
    setComposerText(`${HOST}/${SID}`, '/goal Land the model picker and the mode chip in the header');
  },
};
