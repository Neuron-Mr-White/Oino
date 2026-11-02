// @vitest-environment jsdom
import { waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockHost } from '../dev/mock/host';
import { setHostClientFactory } from './host/factory';
import type { SessionSnapshot } from './host/client';
import roundtrip from './session/fixtures/roundtrip.json';
import { resetHubs } from './session/hub';
import { resetSessions, sessions } from './session/store';
import { installUiSinks, resetUiRegistries, sessionUi } from '../ui/registries';
import {
  RUNNING_NOTIFICATION_ID,
  handlePendingAction,
  resetRunningNotification,
  runningText,
  runningUpdateDue,
  startRunningNotification,
} from './runningNotification';

afterEach(() => {
  resetRunningNotification();
  resetUiRegistries();
  resetSessions();
  resetHubs();
  setHostClientFactory(undefined);
  delete (window as unknown as { UniPiNotify?: unknown }).UniPiNotify;
});

describe('runningText (UNI-161 §5)', () => {
  it('names the activity and the elapsed time', () => {
    expect(runningText({ session: 'auth rewrite', title: 'Running', detail: 'npm test', since: 1_000, now: 186_000 })).toEqual({ title: 'auth rewrite', body: 'Running npm test · 3m 5s' });
  });
  it('works without detail or start time', () => {
    expect(runningText({ session: '', title: 'Thinking', now: 1 })).toEqual({ title: 'Unipi is working', body: 'Thinking' });
  });
});

describe('runningUpdateDue (UNI-223: no re-alert per chat change)', () => {
  const last = { title: 'Running', text: 's\nRunning npm test · 5s', at: 10_000 };
  it('posts the first time', () => {
    expect(runningUpdateDue(undefined, { title: 'Running', text: 'x' }, 0)).toBe(true);
  });
  it('never re-posts identical text (streaming deltas, tool count churn)', () => {
    expect(runningUpdateDue(last, { title: 'Running', text: last.text }, 999_999)).toBe(false);
  });
  it('a new activity goes out right away', () => {
    expect(runningUpdateDue(last, { title: 'Thinking', text: 's\nThinking · 5s' }, 10_001)).toBe(true);
  });
  it('the elapsed-time tick waits the gap', () => {
    expect(runningUpdateDue(last, { title: 'Running', text: 's\nRunning npm test · 6s' }, 11_000)).toBe(false);
    expect(runningUpdateDue(last, { title: 'Running', text: 's\nRunning npm test · 15s' }, 15_000)).toBe(true);
  });
});

describe('v2 session store watcher & pending actions (P0-06)', () => {
  const rt = roundtrip as unknown as { fresh: SessionSnapshot };

  it('posts and cancels the ongoing notification from v2 sessions() and handles Stop + dialog answers', async () => {
    installUiSinks();
    const show = vi.fn();
    const cancel = vi.fn();
    let pendingJson: string | null = null;
    Object.defineProperty(window, 'UniPiNotify', {
      configurable: true,
      writable: true,
      value: {
        show,
        cancel,
        takePendingAction: () => {
          const out = pendingJson;
          pendingJson = null;
          return out;
        },
      },
    });
    const base: SessionSnapshot = {
      ...rt.fresh,
      seq: 0,
      entries: [],
      ui: undefined,
      state: { ...((rt.fresh.state ?? {}) as Record<string, unknown>), sessionName: 'v2-build', isStreaming: false },
    };
    const world = createMockHost({
      name: 'running-notify',
      hosts: [{ id: 'local', name: 'PC', sessions: { s1: { snapshot: base, events: [] } } }],
    });
    setHostClientFactory(world.client);

    const handle = sessions().ensure('local', 's1');
    await waitFor(() => expect(handle.state.conn).toBe('live'));

    const stop = startRunningNotification(() => true);
    try {
      expect(show).not.toHaveBeenCalled();
      world.push('local', 's1', [{ session: 's1', seq: 1, event: { type: 'agent_start' } }]);
      await waitFor(() => expect(show).toHaveBeenCalledWith(RUNNING_NOTIFICATION_ID, 'v2-build', expect.stringContaining('Working'), true, expect.any(String)));

      // Stop action aborts the v2 session.
      pendingJson = JSON.stringify({ notificationId: RUNNING_NOTIFICATION_ID, action: 'stop' });
      expect(handlePendingAction()).toBe(true);
      await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'abort')).toBe(true));

      // Settling clears the ongoing notification.
      world.push('local', 's1', [{ session: 's1', seq: 2, event: { type: 'agent_end', messages: [] } }]);
      await waitFor(() => expect(cancel).toHaveBeenCalledWith(RUNNING_NOTIFICATION_ID));

      // Pending ask_user/select action answers the v2 dialog.
      sessionUi('local', 's1').applyRequest({ type: 'extension_ui_request', id: 'dlg-a', method: 'select', title: 'Pick', options: ['yes', 'no'] });
      pendingJson = JSON.stringify({ notificationId: 1000, action: 'yes' });
      expect(handlePendingAction()).toBe(true);
      await waitFor(() =>
        expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'extension_ui_response' && (e.args as { id?: string; value?: string }).id === 'dlg-a' && (e.args as { value?: string }).value === 'yes')).toBe(true),
      );
    } finally {
      stop();
    }
  });
});
