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
