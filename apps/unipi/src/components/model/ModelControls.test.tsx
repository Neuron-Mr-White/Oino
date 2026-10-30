// @vitest-environment jsdom
/**
 * Composer model controls (WP-A14 acceptance): the model chip (Z-69/Z-70),
 * the separate thinking chip with its level menu (Z-71), the context ring
 * visible at 0 % and opening Session → Context (Z-73), and the A14 chords in
 * the shortcut registry (Ctrl/Cmd+M, Ctrl/Cmd+T, Ctrl/Cmd+Shift+M).
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { resetSessions, sessions } from '../../lib/session/store';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { bindingForEvent } from '../../lib/shortcuts/registry';
import { resetThinkingLevels } from '../../lib/model/thinking';
import { resetComposerDrafts } from '../../lib/composer/drafts';
import { Composer } from '../composer/Composer';
import { createMockHost, type MockScenario, type MockWorld } from '../../dev/mock/host';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { sessionModal } from '../session/modalTarget';
import { ModelChip } from './ModelChip';
import { ThinkingChip } from './ThinkingChip';
import { ContextRing } from './ContextRing';
import { closeModelPicker, modelPickerTarget } from './target';

const HOST = 'this-pc';
const SESSION = 's001';
const tick = () => new Promise((r) => setTimeout(r, 0));

async function boot(scenario: MockScenario): Promise<MockWorld> {
  const world = createMockHost({ ...scenario, speed: 0 });
  setHostClientFactory(world.client);
  scenario.setup?.();
  await tick();
  return world;
}

/** The booted scenario's session, opened and live. */
async function open(name: keyof typeof SCENARIOS): Promise<MockWorld> {
  const world = await boot(SCENARIOS[name]!);
  const handle = sessions().ensure(HOST, SESSION);
  await waitFor(() => expect(handle.state.conn).toBe('live'));
  return world;
}

/** A copy of a scenario whose session runs `provider/id` (and answers its levels). */
function withModel(scenario: MockScenario, model: { provider: string; id: string; name: string }): MockScenario {
  return {
    ...scenario,
    hosts: scenario.hosts.map((h) => ({
      ...h,
      sessions: h.sessions
        ? Object.fromEntries(
            Object.entries(h.sessions).map(([id, s]) => [
              id,
              { ...s, snapshot: { ...s.snapshot, state: { ...(s.snapshot.state as object), model } } },
            ]),
          )
        : undefined,
    })),
  };
}

const cmdArgs = (world: MockWorld, op: string) => world.log.filter((l) => l.kind === 'cmd' && l.op === op).map((l) => l.args as Record<string, unknown>);

beforeEach(() => setDesktopOverride(true));
afterEach(() => {
  cleanup();
  resetSessions();
  resetHubs();
  resetLayout();
  resetThinkingLevels();
  resetComposerDrafts();
  closeModelPicker();
  setHostClientFactory(undefined);
  window.localStorage.clear();
});

describe('ModelChip (Z-69/Z-70)', () => {
  it('shows the running model and opens the picker', async () => {
    await open('model-picker');
    const s = sessions().ensure(HOST, SESSION);
    render(() => <ModelChip session={s} />);
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toBe('Claude Sonnet 4.5'));
    fireEvent.click(screen.getByTestId('model-chip'));
    expect(modelPickerTarget()).toEqual({ hostId: HOST, sessionId: SESSION });
  });
});

describe('ThinkingChip (Z-71)', () => {
  it('shows the level and sends set_thinking_level from its menu', async () => {
    const world = await open('thinking-chip');
    const s = sessions().ensure(HOST, SESSION);
    render(() => <ThinkingChip session={s} />);
    await waitFor(() => expect(screen.getByTestId('thinking-chip-label').textContent).toBe('High'));
    fireEvent.click(screen.getByTestId('thinking-chip'));
    const menu = await screen.findByRole('menu');
    expect(menu.textContent).toContain('Off');
    expect(menu.textContent).toContain('Minimal');
    expect(menu.textContent).toContain('High');
    fireEvent.click([...menu.querySelectorAll('[role="menuitem"]')].find((r) => r.textContent === 'Low')!);
    await waitFor(() => expect(cmdArgs(world, 'set_thinking_level')).toHaveLength(1));
    expect(cmdArgs(world, 'set_thinking_level')[0]).toMatchObject({ level: 'low' });
    // the host's thinking_level_changed event follows into the store
    await waitFor(() => expect(s.state.thinkingLevel).toBe('low'));
  });

  it('is hidden for a model without reasoning levels', async () => {
    await boot(withModel(SCENARIOS['thinking-chip']!, { provider: 'zai', id: 'glm-4.5-air', name: 'GLM-4.5 Air' }));
    const s = sessions().ensure(HOST, SESSION);
    await waitFor(() => expect(s.state.conn).toBe('live'));
    render(() => <ThinkingChip session={s} />);
    await waitFor(() => expect(s.state.model?.id).toBe('glm-4.5-air'));
    await tick();
    expect(screen.queryByTestId('thinking-chip')).toBeNull();
  });
});

describe('ContextRing (Z-73)', () => {
  it('draws the percentage and opens Session → Context on click', async () => {
    await open('thinking-chip');
    const s = sessions().ensure(HOST, SESSION);
    render(() => <ContextRing session={s} />);
    await waitFor(() => expect(screen.getByTestId('context-ring').getAttribute('data-percent')).toBe('37'));
    expect(screen.getByTestId('context-ring').getAttribute('aria-label')).toContain('37%');
    fireEvent.click(screen.getByTestId('context-ring'));
    expect(sessionModal()).toMatchObject({ hostId: HOST, sessionId: SESSION, tab: 'context' });
  });

  it('stays visible at 0 % (QA#8): a track and a 2 % arc floor', async () => {
    const scenario = SCENARIOS['thinking-chip']!;
    const empty: MockScenario = {
      ...scenario,
      hosts: scenario.hosts.map((h) => ({
        ...h,
        sessions: h.sessions
          ? Object.fromEntries(
              Object.entries(h.sessions).map(([id, s]) => [
                id,
                { ...s, cmd: { ...s.cmd, '_oino/get_context_usage': () => ({ tokens: 0, contextWindow: 200_000, percent: 0 }) } },
              ]),
            )
          : undefined,
      })),
    };
    await boot(empty);
    const s = sessions().ensure(HOST, SESSION);
    render(() => <ContextRing session={s} />);
    await waitFor(() => expect(screen.getByTestId('context-ring').getAttribute('aria-label')).toContain('0%'));
    await waitFor(() => expect(screen.getByTestId('context-ring').getAttribute('data-empty')).toBe(''));
    const ring = screen.getByTestId('context-ring');
    expect(ring.querySelector('.ctx-ring-track')).toBeTruthy();
    const arc = ring.querySelector('.ctx-ring-arc');
    expect(arc).toBeTruthy();
    // 2 % of the circumference is drawn, so the ring never disappears
    const dash = Number.parseFloat((arc as SVGElement).getAttribute('stroke-dasharray') ?? '0');
    expect(dash).toBeGreaterThan(0);
    expect(ring.getAttribute('aria-label')).toContain('0%');
  });
});

describe('/model (Z-70)', () => {
  it('opens the picker from the composer and sends nothing', async () => {
    const world = await open('thinking-chip');
    const s = sessions().ensure(HOST, SESSION);
    const view = render(() => <Composer session={s} cwd={() => '/home/you/projects/unipi'} />);
    const field = view.container.querySelector('.chip-field') as HTMLDivElement;
    field.focus();
    field.textContent = '/model';
    fireEvent.input(field);
    fireEvent.keyDown(field, { key: 'Enter' });
    await waitFor(() => expect(modelPickerTarget()).toEqual({ hostId: HOST, sessionId: SESSION }));
    expect(world.log.filter((l) => l.kind === 'cmd' && l.op === 'prompt')).toHaveLength(0);
  });
});

describe('A14 shortcuts (Z-69..Z-74)', () => {
  const press = (key: string, shift = false) =>
    bindingForEvent({ key, code: `Key${key.toUpperCase()}`, ctrlKey: true, metaKey: false, altKey: false, shiftKey: shift });

  it('Ctrl/Cmd+M opens the model, Ctrl/Cmd+T cycles thinking, Ctrl/Cmd+Shift+M cycles the mode', () => {
    expect(press('m')).toEqual({ kind: 'app', id: 'model.open' });
    expect(press('t')).toEqual({ kind: 'app', id: 'thinking.cycle' });
    expect(press('m', true)).toEqual({ kind: 'app', id: 'mode.cycle' });
  });
});
