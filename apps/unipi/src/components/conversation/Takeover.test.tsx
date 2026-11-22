// @vitest-environment jsdom
/**
 * Session takeover (WP-A9, Z-51): the host's `taken_over` event turns the
 * conversation read-only — a banner naming the source and **Take back** — and
 * the composer stops accepting input until Take back reopens the session
 * (`session_open` on the mock's log).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { genCards, genFsTree, genSession } from '../../dev/mock/gen';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { resetHubs } from '../../lib/session/hub';
import { resetPanes } from '../../lib/panes';
import { resetUiRegistries } from '../../ui/registries';
import { Root } from '../../main';
import { isTakenOverReadOnly, takeoverSource } from './ReadOnlyBanner';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const HOST = 'pc-coffee';
const SESSION = 's001';
const CWD = '/home/you/projects/unipi';

function takenOverScenario(): MockScenario {
  const base = genSession(SESSION, { seed: 4, turns: 2, toolsPerTurn: 1, cwd: CWD });
  return {
    name: 'taken-over',
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        conn: { state: 'connected' },
        cards: genCards(3, { seed: 5 }),
        fs: genFsTree(CWD),
        sessions: { [SESSION]: { snapshot: base.snapshot, events: [{ at: 0, dto: { session: SESSION, seq: 1, event: { kind: 'taken_over', from: 'tui' } } }] } },
      },
    ],
    speed: 0,
  };
}

afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetPanes();
  resetLayout();
  resetUiRegistries();
  localStorage.clear();
  location.hash = '';
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe('takeover naming', () => {
  it('names the wire source in the sentence', () => {
    expect(takeoverSource('tui')).toBe('the terminal');
    expect(takeoverSource('cli')).toBe('the CLI');
    expect(takeoverSource('host:coffee')).toBe('coffee');
    expect(takeoverSource(undefined)).toBe('another window');
  });

  it('only a taken_over banner is read-only', () => {
    expect(isTakenOverReadOnly({ banner: { kind: 'taken_over', text: 'x', at: 1 } })).toBe(true);
    expect(isTakenOverReadOnly({ banner: { kind: 'crashed', text: 'x', at: 1 } })).toBe(false);
    expect(isTakenOverReadOnly({ banner: undefined })).toBe(false);
  });
});

describe('taken over while open', () => {
  it('goes read-only with a banner, and Take back reopens the session', async () => {
    setPlatformOverride('tauri-desktop');
    setDesktopOverride(true);
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 });
    (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: () => Promise.resolve(0) };
    const world = createMockHost(takenOverScenario());
    setHostClientFactory(world.client);
    location.hash = `#/s/${HOST}/${SESSION}`;
    render(() => <Root />);

    const banner = await screen.findByTestId('tl-readonly-banner');
    expect(banner).toHaveTextContent('Taken over from the terminal.');
    // The composer is disabled while another runner holds the session.
    const field = (await screen.findByTestId('tl-composer')).querySelector('.chip-field') as HTMLDivElement;
    await waitFor(() => expect(field.getAttribute('contenteditable')).toBe('false'));

    const opens = () => world.log.filter((e) => e.kind === 'open' && e.op === 'session_open').length;
    const before = opens();
    fireEvent.click(screen.getByTestId('tl-take-back'));
    await waitFor(() => expect(opens()).toBe(before + 1));
    await waitFor(() => expect(screen.queryByTestId('tl-readonly-banner')).toBeNull());
    await waitFor(() => expect(field.getAttribute('contenteditable')).toBe('true'));
  });
});
