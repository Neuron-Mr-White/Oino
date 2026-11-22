// @vitest-environment jsdom
/**
 * Working-mode chip (WP-A14 acceptance, Z-74): the header switch shows the
 * running mode with its live count, its menu starts Goal / Ralph / Graph by
 * prefilling the composer (no promptless switch exists), Plain sends
 * `/regular`, and the chip is gone when unipi.long-horizon is off.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { resetSessions, sessions } from '../../lib/session/store';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { composerDraft, resetComposerDrafts, setComposerText } from '../../lib/composer/drafts';
import { createMockHost, type MockScenario, type MockWorld } from '../../dev/mock/host';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { ModeChip } from './ModeChip';

const HOST = 'this-pc';
const SESSION = 's001';
const KEY = `${HOST}/${SESSION}`;
const tick = () => new Promise((r) => setTimeout(r, 0));

async function boot(scenario: MockScenario): Promise<MockWorld> {
  const world = createMockHost({ ...scenario, speed: 0 });
  setHostClientFactory(world.client);
  scenario.setup?.();
  await tick();
  return world;
}

/** The same scenario without `unipi.long-horizon` in its extension list. */
function withoutLh(scenario: MockScenario): MockScenario {
  return {
    ...scenario,
    hosts: scenario.hosts.map((h) => ({
      ...h,
      sessions: h.sessions
        ? Object.fromEntries(Object.entries(h.sessions).map(([id, s]) => [id, { ...s, cmd: { ...s.cmd, '_oino/extensions': () => ({ extensions: [] }) } }]))
        : undefined,
    })),
  };
}

const promptMessages = (world: MockWorld) =>
  world.log.filter((l) => l.kind === 'cmd' && l.op === 'prompt').map((l) => (l.args as { message: string }).message);

/** Graph mode with no graph yet: the chip shows (with a graph the capsule owns the entry). */
async function withoutGraph(world: MockWorld) {
  const seq = sessions().peek(HOST, SESSION)!.state.seq;
  world.push(HOST, SESSION, [{ session: SESSION, seq: seq + 1, event: { type: 'topic', topic: 'unipi:long-horizon:progress', data: { mode: 'graph', goal: null, ralph: null, graph: null }, sticky: true, publisher: 'unipi.long-horizon' } }]);
  await tick();
}

beforeEach(() => setDesktopOverride(true));
afterEach(() => {
  cleanup();
  resetSessions();
  resetHubs();
  resetLayout();
  resetComposerDrafts();
  setHostClientFactory(undefined);
  window.localStorage.clear();
});

describe('ModeChip (Z-74)', () => {
  it('shows the running mode with its pulse', async () => {
    const world = await boot(SCENARIOS['mode-chip']!);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
    await tick();
    await withoutGraph(world);
    await waitFor(() => expect(screen.getByTestId('mode-chip')).toBeTruthy());
    expect(screen.getByTestId('mode-chip').getAttribute('data-mode')).toBe('graph');
    expect(screen.getByTestId('mode-chip-label').textContent).toBe('Graph');
  });

  it('starts Goal / Ralph / Graph as starter commands in the composer', async () => {
    const world = await boot(SCENARIOS['mode-chip']!);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
    await tick();
    await withoutGraph(world);
    await waitFor(() => expect(screen.getByTestId('mode-chip')).toBeTruthy());
    setComposerText(KEY, '');
    fireEvent.click(screen.getByTestId('mode-chip'));
    const menu = await screen.findByRole('menu');
    expect(menu.textContent).toContain('Start');
    fireEvent.click(screen.getByTestId('mode-start-goal'));
    await tick();
    expect(composerDraft(KEY).text).toBe('/goal ');
    // and the chip's own starter keeps the words already typed
    setComposerText(KEY, 'ship the picker');
    fireEvent.click(screen.getByTestId('mode-chip'));
    await screen.findByRole('menu');
    fireEvent.click(screen.getByTestId('mode-start-ralph'));
    await tick();
    expect(composerDraft(KEY).text).toBe('/ralph start ship the picker');
    // nothing was sent: a starter only prepares the message
    expect(promptMessages(world)).toEqual([]);
  });

  it('Plain stops the mode with /regular', async () => {
    const world = await boot(SCENARIOS['mode-chip']!);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
    await tick();
    await withoutGraph(world);
    await waitFor(() => expect(screen.getByTestId('mode-chip')).toBeTruthy());
    fireEvent.click(screen.getByTestId('mode-chip'));
    await screen.findByRole('menu');
    fireEvent.click(screen.getByTestId('mode-plain'));
    await waitFor(() => expect(promptMessages(world)).toContain('/regular'));
  });

  // owner 2026-11-04: the header "Graph 6/12" button was redundant with the status capsule —
  // while a graph runs the chip is gone (the capsule's Graph segment opens the graph).
  it('hides while graph mode has a graph (the capsule opens it)', async () => {
    await boot(SCENARIOS['mode-chip']!);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} cwd="/home/me/proj" />);
    await tick();
    await tick();
    expect(screen.queryByTestId('mode-chip')).toBeNull();
    expect(screen.queryByTestId('mode-chip-menu')).toBeNull();
  });

  it('keeps the menu on the chip while graph mode has no graph yet', async () => {
    const world = await boot(SCENARIOS['mode-chip']!);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
    await tick();
    let seq = sessions().peek(HOST, SESSION)!.state.seq;
    world.push(HOST, SESSION, [{ session: SESSION, seq: ++seq, event: { type: 'topic', topic: 'unipi:long-horizon:progress', data: { mode: 'graph', goal: null, ralph: null, graph: null }, sticky: true, publisher: 'unipi.long-horizon' } }]);
    await waitFor(() => expect(screen.getByTestId('mode-chip').getAttribute('data-opens')).toBeNull());
    fireEvent.click(screen.getByTestId('mode-chip'));
    expect(await screen.findByRole('menu')).toBeTruthy();
  });

  // [graph-mode] P0: `/graph <task>` mid-chat must light the chip from the extension's mode
  // event (the sticky `unipi:long-horizon:state` topic), and `/regular` must clear it.
  it('follows the mode event: Plain → Graph → Plain', async () => {
    const world = await boot(SCENARIOS['mode-chip']!);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
    await tick();
    let seq = sessions().peek(HOST, SESSION)!.state.seq;
    const topic = (name: string, data: unknown) => ({ session: SESSION, seq: ++seq, event: { type: 'topic', topic: name, data, sticky: true, publisher: 'unipi.long-horizon' } });
    const plain = { mode: 'none', goal: null, ralph: null, graph: null, todos: 'no plan' };
    world.push(HOST, SESSION, [topic('unipi:long-horizon:state', { mode: 'none' }), topic('unipi:long-horizon:progress', plain)]);
    await waitFor(() => expect(screen.getByTestId('mode-chip-label').textContent).toBe('Plain'));
    expect(screen.getByTestId('mode-chip').getAttribute('data-mode')).toBe('none');
    // `/graph <task>`: long-horizon publishes the switch right away (before the turn starts).
    world.push(HOST, SESSION, [topic('unipi:long-horizon:state', { mode: 'graph' }), topic('unipi:long-horizon:progress', { ...plain, mode: 'graph' })]);
    await waitFor(() => expect(screen.getByTestId('mode-chip-label').textContent).toBe('Graph'));
    const chip = screen.getByTestId('mode-chip');
    expect(chip.getAttribute('data-mode')).toBe('graph');
    expect(chip.getAttribute('data-phase')).toBe('selected');
    // `/regular`
    world.push(HOST, SESSION, [topic('unipi:long-horizon:state', { mode: 'none' })]);
    await waitFor(() => expect(screen.getByTestId('mode-chip-label').textContent).toBe('Plain'));
  });

  // [graph-mode] version skew: an agent host older than the app has no `_oino/topics`; the chip
  // says the mode can't be shown instead of a silent "Plain" that a /graph never changes.
  it('says so when the agent host is too old to report the mode', async () => {
    const base = SCENARIOS['mode-chip']!;
    const old: MockScenario = {
      ...base,
      hosts: base.hosts.map((h) => ({
        ...h,
        sessions: h.sessions
          ? Object.fromEntries(
              Object.entries(h.sessions).map(([id, s]) => [
                id,
                {
                  ...s,
                  ...(s.snapshot ? { snapshot: { ...s.snapshot, topics: [] } } : {}),
                  cmd: {
                    ...s.cmd,
                    '_oino/topics': () => {
                      throw { code: 'E_INTERNAL', message: 'Unknown command: _oino/topics', retryable: false };
                    },
                  },
                },
              ]),
            )
          : undefined,
      })),
    };
    await boot(old);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
    await waitFor(() => expect(screen.getByTestId('mode-chip-label').textContent).toBe('Mode ?'));
    const chip = screen.getByTestId('mode-chip');
    expect(chip.getAttribute('data-phase')).toBe('unsupported');
    expect(chip.getAttribute('title')).toContain('older than the app');
    fireEvent.click(chip);
    await screen.findByRole('menu');
    expect(screen.getByTestId('mode-switch-desc').textContent).toContain('Update Unipi on that PC');
  });

  it('is hidden when unipi.long-horizon is off (no dead control)', async () => {
    const world = await boot(withoutLh(SCENARIOS['mode-chip']!));
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
    await waitFor(() => expect(world.log.some((l) => l.op === '_oino/extensions')).toBe(true));
    await tick();
    expect(screen.queryByTestId('mode-chip')).toBeNull();
    expect(sessions().peek(HOST, SESSION)).toBeTruthy();
  });
});
