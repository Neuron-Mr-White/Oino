// @vitest-environment jsdom
/**
 * [graph-view] the graph inspector's rows: each node shows its status (a spinner
 * while running), a muted `profile · model · thinking` second line from the run
 * record, and the failed / blocked / superseded styles.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
vi.mock('../../lib/markdown/mermaid', () => ({ renderMermaid: vi.fn(async () => ({ ok: false, error: 'jsdom' })) }));
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetSessions, sessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { createMockHost } from '../../dev/mock/host';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { sessionUi } from '../../ui/registries';
import { LH_VISUALIZE_ID, LhGraph } from './LhGraph';

const HOST = 'this-pc';
const SESSION = 's001';
const tick = () => new Promise((r) => setTimeout(r, 0));

const NODES = [
  { id: 'survey', status: 'completed', instruction: 'Survey the API', profile: 'subagent_explore', model: 'zai/glm-5.3', thinking: 'low', inputs: [] },
  { id: 'client', status: 'running', instruction: 'Write the client', profile: 'rel-qa', model: 'zai/glm-5.3', thinking: 'high', inputs: ['r-survey'] },
  { id: 'docs', status: 'failed', instruction: 'Write the docs', profile: 'subagent_general', model: 'cc/claude-opus-5-5', thinking: 'medium', failure: 'permanent: boom', inputs: [] },
  { id: 'ship', status: 'blocked', instruction: 'Ship it', profile: 'subagent_general', inputs: ['r-docs'] },
  { id: 'old', status: 'superseded', instruction: 'Old plan', profile: 'subagent_general', supersededBy: 'client', inputs: [] },
];

function seedEntry() {
  sessionUi(HOST, SESSION).applySnapshot({
    scope: 'session',
    sessionId: SESSION,
    rev: 1,
    dialogs: [],
    entries: [
      {
        id: LH_VISUALIZE_ID,
        extensionId: 'unipi.long-horizon',
        scope: 'session',
        slot: 'sidePane',
        order: 0,
        seq: 1,
        rev: 1,
        view: {
          kind: 'reserved',
          component: 'lhStatus',
          params: { v: 1, graph: { id: 'g-1', mode: 'graph', task: 'T', state: 'open' }, nodes: NODES },
          fallback: { t: 'Layout', children: [{ t: 'Markdown', source: NODES.map((n) => `- ${n.id} [${n.status}] ${n.instruction}`).join('\n') }] },
        },
      },
    ],
  });
}

beforeEach(() => setDesktopOverride(true));
afterEach(() => {
  cleanup();
  resetSessions();
  resetHubs();
  resetLayout();
  setHostClientFactory(undefined);
  window.localStorage.clear();
});

describe('[graph-view] LhGraph rows', () => {
  it('renders profile · model · thinking per node, a spinner for running and the state styles', async () => {
    const world = createMockHost({ ...SCENARIOS['status-lh-graph']!, speed: 0 });
    setHostClientFactory(world.client);
    await tick();
    const s = sessions().ensure(HOST, SESSION);
    const release = s.acquire();
    render(() => <LhGraph s={s} counts="{}" />);
    // Seed the answer once the stream is live (its UI snapshot would replace it).
    await waitFor(() => expect(world.log.some((l) => l.kind === 'cmd' && l.op === 'prompt')).toBe(true));
    await new Promise((r) => setTimeout(r, 30));
    seedEntry();
    await waitFor(() => expect(screen.getAllByTestId('lh-node')).toHaveLength(5));
    const rows = screen.getAllByTestId('lh-node');
    const run = (i: number) => rows[i]!.querySelector('[data-testid=lh-node-run]')?.textContent;
    expect(run(0)).toBe('explore · GLM 5.3 · low');
    expect(run(1)).toBe('rel-qa · GLM 5.3 · high');
    expect(run(2)).toBe('general · Claude Opus 5.5 · medium');
    // Not run yet: the declared profile only — no model guessed from it.
    expect(run(3)).toBe('general');
    expect(rows[1]!.querySelector('[data-testid=lh-node-spinner]')).toBeTruthy();
    expect(rows[0]!.querySelector('[data-testid=lh-node-spinner]')).toBeNull();
    expect(rows.map((r) => r.getAttribute('data-state'))).toEqual(['done', 'running', 'failed', 'blocked', 'off']);
    expect(rows[3]!.querySelector('.lh-dot')?.getAttribute('data-state')).toBe('blocked');
    expect(rows[2]!.querySelector('[data-testid=lh-node-run]')?.getAttribute('title')).toContain('Model: cc/claude-opus-5-5');
    // It asked the host for the graph.
    expect(world.log.some((l) => l.kind === 'cmd' && l.op === 'prompt' && (l.args as { message: string }).message === '/visualize-progress')).toBe(true);
    release();
  });

  it('still lists an older host tree (no params, no run line)', async () => {
    const world = createMockHost({ ...SCENARIOS['status-lh-graph']!, speed: 0 });
    setHostClientFactory(world.client);
    await tick();
    const s = sessions().ensure(HOST, SESSION);
    sessionUi(HOST, SESSION).applySnapshot({
      scope: 'session',
      sessionId: SESSION,
      rev: 1,
      dialogs: [],
      entries: [
        {
          id: LH_VISUALIZE_ID,
          extensionId: 'unipi.long-horizon',
          scope: 'session',
          slot: 'sidePane',
          order: 0,
          seq: 1,
          rev: 1,
          view: { kind: 'reserved', component: 'lhStatus', fallback: { t: 'Layout', children: [{ t: 'Markdown', source: '- w1 [running] Survey\n- w2 [queued] Client' }] } },
        },
      ],
    });
    render(() => <LhGraph s={s} counts="{}" />);
    await waitFor(() => expect(screen.getAllByTestId('lh-node')).toHaveLength(2));
    expect(screen.queryAllByTestId('lh-node-run')).toHaveLength(0);
  });
});
