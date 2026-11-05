// @vitest-environment jsdom
/**
 * The in-chat model picker (WP-A14 acceptance, Z-69/Z-76) — the ONE model
 * dialog (the full "Model" dialog) with this chat's data: the Now line in sentence
 * case, one search filtering the Single list *and* both Fusion columns, the
 * modality icons / context badges with the current row checked, and the one explicit
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
import { ChatModelPicker, ModelPickerLayer } from './ChatModelPicker';
import { closeModelPicker, modelPickerTarget, openModelPicker } from './target';

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

/** The scenarios' host settings as shipped (the mock writes `set_setting` into the scenario's object). */
const PRISTINE = new Map(Object.entries(SCENARIOS).map(([k, v]) => [k, v!.hosts.map((h) => structuredClone(h.settings))]));
/** A scenario whose host settings are a fresh copy of the shipped ones. */
async function bootFresh(name: keyof typeof SCENARIOS): Promise<MockWorld> {
  const scenario = SCENARIOS[name]!;
  const settings = PRISTINE.get(name as string)!;
  const world = createMockHost({ ...scenario, hosts: scenario.hosts.map((h, i) => ({ ...h, settings: structuredClone(settings[i]) })), speed: 0 });
  setHostClientFactory(world.client);
  await tick();
  return world;
}

const picker = () => render(() => <ChatModelPicker target={{ hostId: HOST, sessionId: SESSION }} />);
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

describe('the chat model dialog (Z-69)', () => {
  it('shows Now in sentence case and the catalogue with modality icons + context badges', async () => {
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-now').textContent).toContain('Claude Sonnet 4.5'));
    const now = screen.getByTestId('model-picker-now').textContent ?? '';
    expect(now.startsWith('Now')).toBe(true);
    expect(now).not.toContain('NOW');
    const list = screen.getByTestId('model-picker-list');
    expect(list.textContent).toContain('Claude Sonnet 4.5');
    // Input modalities are icons now (no "Vision" text badge).
    expect(list.textContent).not.toContain('Vision');
    expect(list.querySelector('[data-key="anthropic/claude-sonnet-4-5"] [aria-label="Accepts images"]')).toBeTruthy();
    expect(list.querySelector('[data-key="openai/o3"] [aria-label="Accepts images"]')).toBeNull();
    expect(list.querySelector('[data-key="openai/o3"] [aria-label="Accepts text"]')).toBeTruthy();
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
    // D-07: one click switches — no second "Switch model" step.
    fireEvent.click(screen.getByText('GPT-5'));
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
    await waitFor(() => expect(screen.getByTestId('model-tab-fusion').getAttribute('aria-selected')).toBe('true'));
    // the Single tab is the way back to the one list
    expect(screen.getByTestId('model-tab-single').getAttribute('aria-selected')).toBe('false');
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
    expect(screen.queryByTestId('model-tab-single')).toBeNull();
  });
});

// ── the per-row ⋯ menu ──────────────────────────────────────────────────
const rowOf = (key: string) => screen.getByTestId('model-picker-list').querySelector<HTMLElement>(`.mp-row[data-key="${key}"]`)!;
const moreOf = (key: string) => rowOf(key).parentElement!.querySelector<HTMLElement>('[data-testid="model-row-more"]')!;
async function openMenu(key: string) {
  fireEvent.click(moreOf(key));
  await waitFor(() => expect(screen.getByTestId('model-row-menu').dataset.key).toBe(key));
  return screen.getByTestId('model-row-menu');
}
const setPairCalls = (world: MockWorld) =>
  cmds(world, '_oino/service_call').filter((l) => (l.args as { name?: string }).name === 'fusion.set-pair');

describe('the row ⋯ menu', () => {
  it('sets the Fusion lead / sidekick with the Fusion tab\'s write and checks the current roles', async () => {
    const world = await boot('model-fusion');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-tab-single')).toBeTruthy());
    fireEvent.click(screen.getByTestId('model-tab-single'));
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    // the running pair's lead is checked on its row's menu
    let menu = await openMenu('anthropic/claude-opus-4-1');
    expect(menu.querySelector('[data-testid="model-menu-lead"]')!.getAttribute('aria-checked')).toBe('true');
    expect(menu.querySelector('[data-testid="model-menu-sidekick"]')!.getAttribute('aria-checked')).toBe('false');
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByTestId('model-row-menu')).toBeNull());

    menu = await openMenu('openai/gpt-5');
    fireEvent.click(menu.querySelector('[data-testid="model-menu-lead"]')!);
    await waitFor(() => expect(setPairCalls(world)).toHaveLength(1));
    expect((setPairCalls(world)[0]!.args as { payload?: unknown }).payload).toMatchObject({
      active: { kind: 'fusion', lead: 'openai/gpt-5', sidekick: 'zai/glm-4.6' },
    });
    await waitFor(() => expect(toasts().list().some((t) => t.message === 'Fusion pair: GPT-5 ◆ GLM-4.6')).toBe(true));
    // the dialog stays open
    expect(screen.getByTestId('model-picker-list')).toBeTruthy();

    menu = await openMenu('openai/gpt-5-mini');
    fireEvent.click(menu.querySelector('[data-testid="model-menu-sidekick"]')!);
    await waitFor(() => expect(setPairCalls(world)).toHaveLength(2));
    expect((setPairCalls(world)[1]!.args as { payload?: unknown }).payload).toMatchObject({
      active: { kind: 'fusion', lead: 'openai/gpt-5', sidekick: 'openai/gpt-5-mini' },
    });
  });

  it('has no Fusion items when the extension is off', async () => {
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    const menu = await openMenu('openai/gpt-5');
    expect(menu.querySelector('[data-testid="model-menu-lead"]')).toBeNull();
    expect(menu.querySelector('[data-testid="model-menu-sidekick"]')).toBeNull();
    expect(menu.querySelector('[data-testid="model-menu-cycle"]')).toBeTruthy();
    expect(menu.querySelector('[data-testid="model-menu-startup"]')).toBeTruthy();
  });

  it('adds to and removes from the cycle set (`enabledModels`, custom patterns kept)', async () => {
    const world = await bootFresh('model-cycle-set');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    await waitFor(() => expect(rowOf('anthropic/claude-sonnet-4-5')).toBeTruthy());
    let menu = await openMenu('anthropic/claude-sonnet-4-5');
    const remove = menu.querySelector<HTMLElement>('[data-testid="model-menu-cycle"]')!;
    expect(remove.textContent).toContain('Remove from cycle set');
    fireEvent.click(remove);
    await waitFor(() => expect(cmds(world, '_oino/set_setting').length).toBe(1));
    expect(cmds(world, '_oino/set_setting')[0]!.args).toMatchObject({ key: 'enabledModels', value: ['anthropic/*'], scope: 'global' });
    menu = await openMenu('openai/gpt-5');
    const add = menu.querySelector<HTMLElement>('[data-testid="model-menu-cycle"]')!;
    expect(add.textContent).toContain('Add to cycle set');
    fireEvent.click(add);
    await waitFor(() => expect(cmds(world, '_oino/set_setting').length).toBe(2));
    expect(cmds(world, '_oino/set_setting')[1]!.args).toMatchObject({ key: 'enabledModels', value: ['anthropic/*', 'openai/gpt-5'] });
    // nothing was picked
    expect(cmds(world, 'set_model')).toHaveLength(0);
  });

  it('"Use at startup" writes that row\'s model as the default', async () => {
    const world = await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    const menu = await openMenu('openai/gpt-5');
    expect(menu.querySelector('[data-testid="model-menu-startup"]')!.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(menu.querySelector('[data-testid="model-menu-startup"]')!);
    await waitFor(() =>
      expect(cmds(world, '_oino/set_setting').map((l) => l.args)).toEqual(
        expect.arrayContaining([
          expect.objectContaining({ key: 'defaultProvider', value: 'openai' }),
          expect.objectContaining({ key: 'defaultModel', value: 'gpt-5' }),
        ]),
      ),
    );
    await waitFor(() => expect(rowOf('openai/gpt-5').textContent).toContain('Startup'));
    expect(cmds(world, 'set_model')).toHaveLength(0);
  });

  it('keyboard: → / Shift+F10 / the context-menu key open it from the row, Esc closes back to the row', async () => {
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    const row = rowOf('openai/gpt-5');
    for (const key of [{ key: 'ArrowRight' }, { key: 'F10', shiftKey: true }, { key: 'ContextMenu' }]) {
      row.focus();
      fireEvent.keyDown(row, key);
      await waitFor(() => expect(screen.getByTestId('model-row-menu')).toBeTruthy());
      expect(screen.getByTestId('model-row-menu').contains(document.activeElement)).toBe(true);
      fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
      await waitFor(() => expect(screen.queryByTestId('model-row-menu')).toBeNull());
      expect(document.activeElement).toBe(row);
      // the dialog is still open
      expect(screen.getByTestId('model-picker-list')).toBeTruthy();
    }
  });
});

describe('the Cycle set (WP-A25, G-15 /scoped-models)', () => {
  it('/scoped-models opens with the "Cycle set" filter on: only members, the ⋯ hint, custom patterns kept', async () => {
    await bootFresh('model-cycle-set');
    render(() => <ChatModelPicker target={{ hostId: HOST, sessionId: SESSION, focus: 'cycle' }} />);
    await waitFor(() => expect(screen.getByTestId('model-filter-cycle').getAttribute('aria-pressed')).toBe('true'));
    await waitFor(() => expect(screen.getByTestId('model-picker-list').querySelectorAll('.mp-row')).toHaveLength(1));
    expect(screen.getByTestId('model-picker-list').querySelector('.mp-row')!.getAttribute('data-key')).toBe('anthropic/claude-sonnet-4-5');
    expect(screen.getByTestId('model-filter-cycle-hint').textContent).toBe('Use ⋯ to add or remove');
    expect(screen.getByTestId('model-filter-cycle').textContent).toContain('1');
    expect(screen.getByTestId('model-filter-cycle-custom').textContent).toContain('anthropic/*');
    // the chip turns the filter off: the whole catalogue again
    fireEvent.click(screen.getByTestId('model-filter-cycle'));
    await waitFor(() => expect(screen.getByTestId('model-picker-list').querySelectorAll('.mp-row').length).toBeGreaterThan(1));
    expect(screen.queryByTestId('model-filter-cycle-hint')).toBeNull();
  });

  it('removing the last member from the filtered list leaves an empty state that says how to add', async () => {
    await bootFresh('model-cycle-set');
    render(() => <ChatModelPicker target={{ hostId: HOST, sessionId: SESSION, focus: 'cycle' }} />);
    await waitFor(() => expect(screen.getByTestId('model-picker-list').querySelectorAll('.mp-row')).toHaveLength(1));
    const menu = await openMenu('anthropic/claude-sonnet-4-5');
    fireEvent.click(menu.querySelector('[data-testid="model-menu-cycle"]')!);
    await waitFor(() => expect(screen.getByTestId('model-filter-cycle-empty').textContent).toContain('Use ⋯ on a model to add it'));
  });

  it('the old collapsible "Cycle set · …" section is gone; members carry a repeat mark', async () => {
    await bootFresh('model-cycle-set');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    expect(screen.queryByTestId('model-picker-cycle')).toBeNull();
    expect(screen.queryByTestId('model-picker-cycle-toggle')).toBeNull();
    expect(screen.queryByTestId('model-picker-cycle-list')).toBeNull();
    // the filter is off by default
    expect(screen.getByTestId('model-filter-cycle').getAttribute('aria-pressed')).toBe('false');
    await waitFor(() => expect(rowOf('anthropic/claude-sonnet-4-5').querySelector('[aria-label="In the cycle set"]')).toBeTruthy());
    expect(rowOf('openai/gpt-5').querySelector('[aria-label="In the cycle set"]')).toBeNull();
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
    // Selected/running model pinned to the very top, then curated Suggested, then rest, without duplicates.
    expect(keys[0]).toBe('anthropic/claude-sonnet-4-5');
    expect(keys.slice(1, 5)).toEqual(['google/gemini-2.5-pro', 'openai/gpt-5', 'anthropic/claude-opus-4-1', 'zai/glm-4.6']);
    expect(new Set(keys).size).toBe(keys.length);
    expect(screen.getByTestId('model-group-selected')).toBeTruthy();
    expect(screen.getByTestId('model-group-all')).toBeTruthy();
    const gpt = list.querySelector<HTMLElement>('[data-key="openai/gpt-5"]')!;
    expect(gpt.textContent).toContain('Lead');
    expect(gpt.textContent).toContain('High');
    expect(list.querySelector('[data-key="zai/glm-4.6"]')!.textContent).toContain('Sidekick');
    // Display name + full provider/id model path shown on every row to distinguish duplicate model names.
    expect(gpt.textContent).toContain('openai/gpt-5');
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
    const apply = await screen.findByText('Turn Fusion off & switch').catch(() => undefined);
    if (apply) fireEvent.click(apply);
    await waitFor(() => expect(cmds(world, 'set_thinking_level').at(-1)?.args).toMatchObject({ level: 'high' }));
  });

  it('"Use at startup" writes the running model as the default (and Fusion\'s startup.model)', async () => {
    const world = await bootCurated();
    picker();
    await waitFor(() => expect(screen.getByTestId('model-group-suggested')).toBeTruthy());
    // D-07: a click on another row switches at once, so "Use at startup" acts on the running model.
    fireEvent.click(screen.getByTestId('model-use-startup'));
    await waitFor(() => expect(screen.getByTestId('model-use-startup').getAttribute('aria-pressed')).toBe('true'));
    expect(screen.getByTestId('model-use-startup').textContent).toBe('Use at startup');
    const sets = cmds(world, '_oino/set_setting').map((l) => l.args as { key: string; value: unknown });
    expect(sets).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: 'defaultProvider', value: 'anthropic' }),
        expect.objectContaining({ key: 'defaultModel', value: 'claude-sonnet-4-5' }),
      ]),
    );
    expect(cmds(world, '_oino/set_extension_setting').at(-1)?.args).toMatchObject({ namespace: 'fusion', key: 'startup.model', value: 'anthropic/claude-sonnet-4-5' });
    expect(screen.getByTestId('model-picker-list').querySelector('[data-key="anthropic/claude-sonnet-4-5"]')!.textContent).toContain('Startup');
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

describe('one dialog (owner: one model picker, the full one)', () => {
  it('is the centred "Model" dialog: title, close, Now, search, Cycle set chip, list, scope line and Use at startup', async () => {
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    const overlay = screen.getByTestId('model-picker-overlay');
    const dialog = screen.getByRole('dialog');
    expect(overlay.contains(dialog)).toBe(true);
    expect(dialog.getAttribute('aria-label')).toBe('Model');
    expect(dialog.querySelector('h2')?.textContent).toBe('Model');
    expect(dialog.contains(screen.getByTestId('model-picker-close'))).toBe(true);
    expect(screen.queryByRole('menu')).toBeNull();
    expect((screen.getByTestId('model-search') as HTMLInputElement).placeholder).toMatch(/^Search all \d+ models$/);
    const foot = screen.getByTestId('model-picker-foot');
    expect(foot.contains(screen.getByTestId('model-use-startup'))).toBe(true);
    expect(screen.getByTestId('model-picker-scope').textContent).toBe('Your pick applies to this chat only. “Use at startup” sets the model new chats start with.');
    expect(foot.contains(screen.getByTestId('model-filter-cycle'))).toBe(false);
    expect(screen.queryByTestId('model-picker-cycle')).toBeNull();
  });

  it('closes with ✕, Esc and a backdrop click; nothing is sent', async () => {
    const world = await boot('model-picker');
    render(() => <ModelPickerLayer />);
    for (const close of [
      () => fireEvent.click(screen.getByTestId('model-picker-close')),
      () => fireEvent.keyDown(screen.getByTestId('model-search'), { key: 'Escape' }),
      () => fireEvent.click(screen.getByTestId('model-picker-overlay')),
    ]) {
      openModelPicker({ hostId: HOST, sessionId: SESSION });
      await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
      close();
      await waitFor(() => expect(screen.queryByTestId('model-picker-overlay')).toBeNull());
      expect(modelPickerTarget()).toBeUndefined();
    }
    expect(cmds(world, 'set_model')).toHaveLength(0);
  });

  it('is a full-screen sheet with the same content on the phone', async () => {
    setDesktopOverride(false);
    await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-dialog-sheet')).toBeTruthy());
    const sheet = screen.getByTestId('model-picker-dialog-sheet');
    await waitFor(() => expect(sheet.contains(screen.getByTestId('model-picker-list'))).toBe(true));
    expect(sheet.contains(screen.getByTestId('model-use-startup'))).toBe(true);
    expect(sheet.contains(screen.getByTestId('model-filter-cycle'))).toBe(true);
    // touch: the ⋯ is on every row
    expect(sheet.querySelectorAll('[data-testid="model-row-more"]').length).toBeGreaterThan(0);
    expect(screen.queryByTestId('model-picker-overlay')).toBeNull();
  });

  it('keyboard: the search has focus, ↓ walks the rows one at a time, Enter in the search picks the first hit', async () => {
    const world = await boot('model-picker');
    picker();
    await waitFor(() => expect(screen.getByTestId('model-picker-list').textContent).toContain('GPT-5'));
    const search = screen.getByTestId('model-search');
    await waitFor(() => expect(document.activeElement).toBe(search));
    const rows = [...screen.getByTestId('model-picker-list').querySelectorAll<HTMLElement>('.mp-row:not(:disabled)')];
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(rows[0]);
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(rows[1]);
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' });
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(search);
    fireEvent.input(search, { target: { value: 'gpt-5' } });
    await tick();
    fireEvent.keyDown(search, { key: 'Enter' });
    await waitFor(() => expect(cmds(world, 'set_model')).toHaveLength(1));
    expect(cmds(world, 'set_model')[0]!.args).toMatchObject({ provider: 'openai', modelId: 'gpt-5' });
  });
});