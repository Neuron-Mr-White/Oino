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
  it('shows the running mode with its live count and a pulse', async () => {
    await boot(SCENARIOS['mode-chip']!);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
    await waitFor(() => expect(screen.getByTestId('mode-chip')).toBeTruthy());
    const chip = screen.getByTestId('mode-chip');
    expect(chip.getAttribute('data-mode')).toBe('graph');
    expect(chip.getAttribute('data-phase')).toBe('active');
    expect(screen.getByTestId('mode-chip-label').textContent).toBe('Graph');
    expect(chip.textContent).toContain('3/6');
    expect(chip.querySelector('.hdr-mode-pulse')).toBeTruthy();
  });

  it('starts Goal / Ralph / Graph as starter commands in the composer', async () => {
    const world = await boot(SCENARIOS['mode-chip']!);
    render(() => <ModeChip hostId={HOST} sessionId={SESSION} />);
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
    await waitFor(() => expect(screen.getByTestId('mode-chip')).toBeTruthy());
    fireEvent.click(screen.getByTestId('mode-chip'));
    await screen.findByRole('menu');
    fireEvent.click(screen.getByTestId('mode-plain'));
    await waitFor(() => expect(promptMessages(world)).toContain('/regular'));
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
