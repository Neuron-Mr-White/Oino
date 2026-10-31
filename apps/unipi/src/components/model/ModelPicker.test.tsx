// @vitest-environment jsdom
/**
 * Model picker (WP-A14 acceptance, Z-69/Z-76): the Now line in sentence
 * case, one search filtering the Single list *and* both Fusion columns, the
 * Vision/context badges with the current row checked, and the one explicit
 * action sending `set_model` (MockHost log) or `fusion.set-pair` plus the
 * "Switched from X to Y" toast (Z-72).
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { resetSessions, sessions } from '../../lib/session/store';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { toasts } from '../../lib/toasts';
import { createMockHost, type MockWorld } from '../../dev/mock/host';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { ModelPicker } from './ModelPicker';
import { closeModelPicker } from './target';

const HOST = 'this-pc';
const SESSION = 's001';
const tick = () => new Promise((r) => setTimeout(r, 0));

async function boot(name: keyof typeof SCENARIOS): Promise<MockWorld> {
  const scenario = SCENARIOS[name]!;
  const world = createMockHost({ ...scenario, speed: 0 });
  setHostClientFactory(world.client);
  scenario.setup?.();
  await tick();
  return world;
}

const picker = () => render(() => <ModelPicker target={{ hostId: HOST, sessionId: SESSION }} />);
const cmds = (world: MockWorld, op: string) => world.log.filter((l) => l.kind === 'cmd' && l.op === op);

beforeEach(() => setDesktopOverride(true));
afterEach(() => {
  cleanup();
  closeModelPicker();
  resetSessions();
  resetHubs();
  resetLayout();
  setHostClientFactory(undefined);
  toasts().clear();
  window.localStorage.clear();
});

describe('the picker sheet (Z-69)', () => {
  it('shows Now in sentence case and the catalogue with Vision + context badges', async () => {
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-now').textContent).toContain('Claude Sonnet 4.5'));
    const now = screen.getByTestId('model-picker-now').textContent ?? '';
    expect(now.startsWith('Now')).toBe(true);
    expect(now).not.toContain('NOW');
    const list = screen.getByTestId('model-picker-list');
    expect(list.textContent).toContain('Claude Sonnet 4.5');
    expect(list.textContent).toContain('Vision');
    expect(list.textContent).toContain('200k');
    expect(list.textContent).toContain('1M');
    // the running model is checked
    expect(list.querySelector('[aria-current="true"]')?.textContent).toContain('Claude Sonnet 4.5');
  });

  it('has one search that spans the whole catalogue', async () => {
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    expect(screen.getByTestId('model-picker-list').textContent).toContain('GPT-5');
    fireEvent.input(screen.getByTestId('model-search'), { target: { value: 'gem' } });
    await tick();
    const list = screen.getByTestId('model-picker-list');
    expect(list.textContent).toContain('Gemini 2.5 Pro');
    expect(list.textContent).not.toContain('GPT-5');
  });

  it('applies a single pick through set_model, closes and toasts (Z-72)', async () => {
    const world = await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list').textContent).toContain('GPT-5'));
    fireEvent.click(screen.getByText('GPT-5'));
    const apply = await screen.findByText('Switch model');
    fireEvent.click(apply);
    await waitFor(() => expect(cmds(world, 'set_model')).toHaveLength(1));
    expect(cmds(world, 'set_model')[0]!.args).toMatchObject({ provider: 'openai', modelId: 'gpt-5' });
    await waitFor(() => expect(toasts().list().some((t) => t.message === 'Switched from Claude Sonnet 4.5 to GPT-5')).toBe(true));
    // the host's model_changed moved the session's selection
    await waitFor(() => expect(sessions().peek(HOST, SESSION)?.state.model?.id).toBe('gpt-5'));
  });
});

describe('Fusion (Z-76)', () => {
  it('opens on the Fusion tab of the running pair and shares the search across columns', async () => {
    await boot('model-fusion');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-now').textContent).toContain('Fusion · Claude Opus 4.1 ◆ GLM-4.6'));
    expect(screen.getByTestId('model-tab-fusion').getAttribute('aria-selected')).toBe('true');
    const fusion = screen.getByTestId('model-picker-fusion');
    const columns = fusion.querySelectorAll('.mp-col-list');
    expect(columns).toHaveLength(2);
    expect(columns[0]!.textContent).toContain('Claude Opus 4.1');
    expect(columns[1]!.textContent).toContain('GLM-4.6');

    fireEvent.input(screen.getByTestId('model-search'), { target: { value: 'glm' } });
    await tick();
    const filtered = screen.getByTestId('model-picker-fusion').querySelectorAll('.mp-col-list');
    for (const column of filtered) {
      expect(column.textContent).toContain('GLM-4.6');
      expect(column.textContent).not.toContain('Claude Opus 4.1');
    }
  });

  it('sets the pair through unipi.fusion and toasts the change', async () => {
    const world = await boot('model-fusion');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-fusion')).toBeTruthy());
    // lead column: pick GPT-5
    const leadColumn = screen.getByTestId('model-picker-fusion').querySelectorAll('.mp-col-list')[0]!;
    fireEvent.click([...leadColumn.querySelectorAll('.mp-row')].find((r) => r.textContent?.includes('GPT-5'))!);
    fireEvent.click(await screen.findByText('Update pair'));
    await waitFor(() => expect(cmds(world, '_oino/service_call').some((l) => (l.args as { name?: string }).name === 'fusion.set-pair')).toBe(true));
    const call = cmds(world, '_oino/service_call').find((l) => (l.args as { name?: string }).name === 'fusion.set-pair')!;
    expect((call.args as { payload?: unknown }).payload).toMatchObject({
      active: { kind: 'fusion', lead: 'openai/gpt-5', sidekick: 'zai/glm-4.6', leadEffort: 'high', sidekickEffort: 'low' },
    });
    await waitFor(() => expect(toasts().list().some((t) => t.message === 'Fusion pair: GPT-5 ◆ GLM-4.6')).toBe(true));
  });

  it('is absent when unipi.fusion is off (no dead tab)', async () => {
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    expect(screen.queryByTestId('model-tab-fusion')).toBeNull();
  });
});

describe('the Cycle set (WP-A25, G-15 /scoped-models)', () => {
  it('opens expanded, shows the stored references and writes `enabledModels`', async () => {
    const world = await boot('model-cycle-set');
    render(() => <ModelPicker target={{ hostId: HOST, sessionId: SESSION, focus: 'cycle' }} />);
    await waitFor(() => expect(screen.getByTestId('model-picker-cycle-toggle')).toBeTruthy());
    const toggle = screen.getByTestId('model-picker-cycle-toggle');
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    // A custom pattern (a glob) is kept as written, not turned into a checkbox.
    await waitFor(() => expect(screen.getByTestId('model-picker-cycle-custom').textContent).toContain('anthropic/*'));
    expect(toggle.textContent).toContain('Cycle set · 1 of');
    const row = screen.getByTestId('cycle-anthropic/claude-sonnet-4-5');
    expect(row.getAttribute('aria-pressed')).toBe('true');
    // Untick it: the setting keeps the glob, the checked reference goes.
    fireEvent.click(row);
    await waitFor(() => expect(cmds(world, '_oino/set_setting').length).toBeGreaterThan(0));
    const call = cmds(world, '_oino/set_setting').at(-1)!;
    expect(call.args).toMatchObject({ key: 'enabledModels', value: ['anthropic/*'], scope: 'global' });
    await waitFor(() => expect(screen.getByTestId('cycle-anthropic/claude-sonnet-4-5').getAttribute('aria-pressed')).toBe('false'));
  });

  it('starts collapsed from the normal picker entry point', async () => {
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-cycle-toggle')).toBeTruthy());
    expect(screen.getByTestId('model-picker-cycle-toggle').getAttribute('aria-expanded')).toBe('false');
    expect(screen.queryByTestId('model-picker-cycle-list')).toBeNull();
  });
});

// ── P0-10: Fusion's curated preset ranks the picker ─────────────────────
const CURATED = {
  lead: [{ key: 'openai/gpt-5', name: 'GPT-5' }, { key: 'anthropic/claude-opus-4-1', name: 'Claude Opus 4.1' }],
  sidekick: [{ key: 'zai/glm-4.6', name: 'GLM-4.6' }],
  ranked: [
    { key: 'google/gemini-2.5-pro', name: 'Gemini 2.5 Pro' },
    { key: 'openai/gpt-5', name: 'GPT-5' },
    { key: 'anthropic/claude-opus-4-1', name: 'Claude Opus 4.1' },
    { key: 'zai/glm-4.6', name: 'GLM-4.6' },
  ],
  default: { lead: 'openai/gpt-5', sidekick: 'zai/glm-4.6' },
  effort: { 'openai/gpt-5': 'high' },
};

async function bootCurated(active: unknown = { kind: 'single', model: 'anthropic/claude-sonnet-4-5' }): Promise<MockWorld> {
  const base = SCENARIOS['model-fusion']!;
  const host0 = base.hosts[0]!;
  const sess = host0.sessions![SESSION]!;
  const orig = sess.cmd!['_oino/service_call']!;
  const spec = {
    ...base,
    hosts: [
      {
        ...host0,
        sessions: {
          [SESSION]: {
            ...sess,
            snapshot: { ...sess.snapshot, state: { ...(sess.snapshot.state as object), model: { provider: 'anthropic', id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5' } } },
            cmd: {
              ...sess.cmd,
              '_oino/service_call': (cmd: Parameters<typeof orig>[0], h: Parameters<typeof orig>[1]) =>
                cmd.name === 'fusion.status' ? { active, status: undefined, curated: CURATED } : orig(cmd, h),
            },
          },
        },
      },
    ],
  };
  const world = createMockHost({ ...spec, speed: 0 });
  setHostClientFactory(world.client);
  await tick();
  return world;
}

describe('Suggested (P0-10, fusion.status.curated)', () => {
  it('ranks `ranked` first with Lead / Sidekick badges and the curated effort, then the whole catalogue', async () => {
    await bootCurated();
    picker();
    await waitFor(() => expect(screen.getByTestId('model-group-suggested')).toBeTruthy());
    const list = screen.getByTestId('model-picker-list');
    const keys = [...list.querySelectorAll<HTMLElement>('.mp-row')].map((r) => r.dataset.key);
    expect(keys.slice(0, 4)).toEqual(['google/gemini-2.5-pro', 'openai/gpt-5', 'anthropic/claude-opus-4-1', 'zai/glm-4.6']);
    // the rest: the running model first, no duplicate of a suggested row
    expect(keys[4]).toBe('anthropic/claude-sonnet-4-5');
    expect(new Set(keys).size).toBe(keys.length);
    expect(screen.getByTestId('model-group-all')).toBeTruthy();
    const gpt = list.querySelector<HTMLElement>('[data-key="openai/gpt-5"]')!;
    expect(gpt.textContent).toContain('Lead');
    expect(gpt.textContent).toContain('High');
    expect(list.querySelector('[data-key="zai/glm-4.6"]')!.textContent).toContain('Sidekick');
    // display names on the row, provider · id in the tooltip, no raw id on the row
    expect(gpt.textContent).not.toContain('openai/gpt-5');
    expect(gpt.title).toContain('openai · gpt-5');
  });

  it('a search filters both groups', async () => {
    await bootCurated();
    picker();
    await waitFor(() => expect(screen.getByTestId('model-group-suggested')).toBeTruthy());
    fireEvent.input(screen.getByTestId('model-search'), { target: { value: 'gpt' } });
    await tick();
    const keys = [...screen.getByTestId('model-picker-list').querySelectorAll<HTMLElement>('.mp-row')].map((r) => r.dataset.key);
    expect(keys).toEqual(['openai/gpt-5', 'openai/gpt-5-mini']);
  });

  it('a suggested pick applies its curated effort with the switch', async () => {
    const world = await bootCurated();
    picker();
    await waitFor(() => expect(screen.getByTestId('model-group-suggested')).toBeTruthy());
    fireEvent.click(screen.getByTestId('model-picker-list').querySelector('[data-key="openai/gpt-5"]')!);
    fireEvent.click(await screen.findByText('Turn Fusion off & switch').catch(() => screen.findByText('Switch model')));
    await waitFor(() => expect(cmds(world, 'set_thinking_level').at(-1)?.args).toMatchObject({ level: 'high' }));
  });

  it('"Use at startup" writes the default model (and Fusion\'s startup.model)', async () => {
    const world = await bootCurated();
    picker();
    await waitFor(() => expect(screen.getByTestId('model-group-suggested')).toBeTruthy());
    fireEvent.click(screen.getByTestId('model-picker-list').querySelector('[data-key="openai/gpt-5"]')!);
    fireEvent.click(screen.getByTestId('model-use-startup'));
    await waitFor(() => expect(screen.getByTestId('model-use-startup').textContent).toBe('Startup model'));
    const sets = cmds(world, '_oino/set_setting').map((l) => l.args as { key: string; value: unknown });
    expect(sets).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: 'defaultProvider', value: 'openai' }),
        expect.objectContaining({ key: 'defaultModel', value: 'gpt-5' }),
        expect.objectContaining({ key: 'defaultThinkingLevel', value: 'high' }),
      ]),
    );
    expect(cmds(world, '_oino/set_extension_setting').at(-1)?.args).toMatchObject({ namespace: 'fusion', key: 'startup.model', value: 'openai/gpt-5' });
    expect(screen.getByTestId('model-picker-list').querySelector('[data-key="openai/gpt-5"]')!.textContent).toContain('Startup');
  });

  it('the Fusion columns put their curated lists first and take the curated default pair', async () => {
    await bootCurated({ kind: 'single', model: 'anthropic/claude-sonnet-4-5' });
    picker();
    await waitFor(() => expect(screen.getByTestId('model-tab-fusion')).toBeTruthy());
    fireEvent.click(screen.getByTestId('model-tab-fusion'));
    await waitFor(() => expect(screen.getByTestId('model-picker-fusion')).toBeTruthy());
    const [lead, side] = [...screen.getByTestId('model-picker-fusion').querySelectorAll('.mp-col-list')];
    const names = (el: Element) => [...el.querySelectorAll('.mp-row-name')].map((n) => n.textContent);
    expect(names(lead!).slice(0, 2)).toEqual(['GPT-5', 'Claude Opus 4.1']);
    expect(names(side!)[0]).toBe('GLM-4.6');
    expect(lead!.querySelector('[aria-current="true"]')?.textContent).toContain('GPT-5');
    expect(side!.querySelector('[aria-current="true"]')?.textContent).toContain('GLM-4.6');
  });
});
