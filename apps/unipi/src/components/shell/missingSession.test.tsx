/**
 * A remembered chat that no longer exists (perf-sessions): the host answers
 * `session_open` with `E_NOT_FOUND` — the app forgets it and lands on New chat
 * instead of re-opening it in a loop (each open used to cost the host a full
 * scan of the sessions folder).
 */
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Root } from '../../main';
import { createMockHost } from '../../dev/mock/host';
import { streamBasic } from '../../dev/mock/scenarios/stream-basic';
import type { HostClient } from '../../lib/host/client';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { resetPanes } from '../../lib/panes';
import { resetHubs } from '../../lib/session/hub';
import { chatGone, forgetLastSession, readLastSession } from './Workspace';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const HOST = 'pc-coffee';
const GONE = '019fcbea-dead-7491-b190-00000000beef';
const LAST_KEY = 'unipi.shell.last';

/** The mock world's client, counting `session_open`s per session. */
function countingFactory() {
  const world = createMockHost(streamBasic);
  const opens = new Map<string, number>();
  const factory = (hostId: string): HostClient => {
    const inner = world.client(hostId);
    return new Proxy(inner, {
      get(target, key, receiver) {
        if (key === 'openSession') {
          return (req: { session?: string }, sink: unknown) => {
            if (req.session) opens.set(req.session, (opens.get(req.session) ?? 0) + 1);
            return (target.openSession as (r: unknown, s: unknown) => unknown).call(target, req, sink);
          };
        }
        const value = Reflect.get(target, key, receiver) as unknown;
        return typeof value === 'function' ? (value as (...a: unknown[]) => unknown).bind(target) : value;
      },
    }) as HostClient;
  };
  return { factory, opens };
}

function boot(hash: string) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 });
  setPlatformOverride('tauri-desktop');
  setDesktopOverride(true);
  const counting = countingFactory();
  setHostClientFactory(counting.factory);
  location.hash = hash;
  render(() => <Root />);
  return counting;
}

beforeEach(() => {
  localStorage.clear();
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: () => Promise.resolve(0) };
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetPanes();
  resetLayout();
  location.hash = '';
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe('a vanished last session', () => {
  it('is forgotten and the app lands on New chat, opened once — no retry loop', async () => {
    localStorage.setItem(LAST_KEY, JSON.stringify({ hostId: HOST, sessionId: GONE }));
    const { opens } = boot('#/');
    // `/` reopens the remembered chat…
    await waitFor(() => expect(opens.get(GONE)).toBe(1));
    // …the host has no such session: New chat, and nothing remembered any more.
    await waitFor(() => expect(location.hash).toBe(`#/new/${HOST}`));
    expect(readLastSession()).toBeUndefined();
    // No retries behind the user's back (the open backoff would have fired by now).
    await new Promise((r) => setTimeout(r, 1_200));
    expect(opens.get(GONE)).toBe(1);
  });

  it('a link to a missing chat lands on New chat too, and is not remembered', async () => {
    const other = streamBasic.hosts[0]!.cards![0]!.session_id;
    localStorage.setItem(LAST_KEY, JSON.stringify({ hostId: HOST, sessionId: other }));
    boot(`#/s/${HOST}/${GONE}`);
    await waitFor(() => expect(location.hash).toBe(`#/new/${HOST}`));
    // The effect remembers the shown chat before it fails; forgetting only drops that one.
    expect(readLastSession()?.sessionId).not.toBe(GONE);
  });
});

describe('chatGone / forgetLastSession', () => {
  const base = { missing: true, live: false, archived: false, listLoaded: true, archivedChecked: true };
  it('needs a not-found answer, no card anywhere and both lists checked', () => {
    expect(chatGone(base)).toBe(true);
    expect(chatGone({ ...base, missing: false })).toBe(false);
    expect(chatGone({ ...base, live: true })).toBe(false);
    // An archived chat opens read-only from the trash: never redirected away.
    expect(chatGone({ ...base, archived: true })).toBe(false);
    expect(chatGone({ ...base, listLoaded: false })).toBe(false);
    expect(chatGone({ ...base, archivedChecked: false })).toBe(false);
  });

  it('forgets only the matching session', () => {
    localStorage.setItem(LAST_KEY, JSON.stringify({ hostId: HOST, sessionId: 'a' }));
    forgetLastSession(HOST, 'b');
    expect(readLastSession()).toEqual({ hostId: HOST, sessionId: 'a' });
    forgetLastSession('other-pc', 'a');
    expect(readLastSession()).toEqual({ hostId: HOST, sessionId: 'a' });
    forgetLastSession(HOST, 'a');
    expect(readLastSession()).toBeUndefined();
  });
});
