// @vitest-environment jsdom
/** UNI-253: a phone can turn a shared feature off / disconnect — "for this
 *  phone" always, "on the PC" only when this device actually has it. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { HashRouter } from '@solidjs/router';
import { LeaveSheet } from './LeaveSheet';
import { setSharingTransport, leaveScopes, isLeaveUnsupported, FEATURE_LABEL, type SharingTransport } from '../lib/sharing';
import { setPlatformOverride } from '../lib/platform';

/** A transport whose `leave` records what it was asked and answers with it. */
function transport(answer: { left: boolean; now_enabled: boolean } | { error: string }): SharingTransport & { calls: unknown[] } {
  const calls: unknown[] = [];
  return {
    calls,
    localHost: async () => ({ ok: true, value: { hostDir: '/x', hostId: 'h' } }),
    status: async () => ({ ok: true, value: {} as never }),
    change: async () => ({ ok: true, value: {} as never }),
    ask: async () => ({ ok: true, value: {} as never }),
    requests: async () => ({ ok: true, value: [] }),
    leave: async (hostId, feature, scope) => {
      calls.push({ hostId, feature, scope });
      return 'error' in answer
        ? { ok: false, error: { code: 'E_SCHEMA', message: answer.error, retryable: false } }
        : { ok: true, value: { feature, left: answer.left, now_enabled: answer.now_enabled, devices: [] } };
    },
  } as SharingTransport & { calls: unknown[] };
}

function sheet(props: Partial<Parameters<typeof LeaveSheet>[0]> = {}) {
  return render(() => (
    <HashRouter>
      <LeaveSheet open hostId="peer-1" feature="forward" allowed onClose={() => {}} {...props} />
    </HashRouter>
  ));
}

beforeEach(() => setPlatformOverride('tauri-mobile'));
afterEach(() => {
  cleanup();
  setSharingTransport(undefined);
  setPlatformOverride(undefined);
});

describe('UNI-253: turn a shared feature off', () => {
  it('offers both scopes when this device has the feature', () => {
    setSharingTransport(transport({ left: true, now_enabled: false }));
    sheet();
    expect(screen.getByTestId('leave-self')).toBeTruthy();
    expect(screen.getByTestId('leave-all')).toBeTruthy();
    // Turning off never enables: the sheet says so up front.
    expect(screen.getByTestId('leave-sheet').textContent).toContain('never turns anything on');
  });

  it('offers only "for this phone" when the device does not have it', () => {
    setSharingTransport(transport({ left: false, now_enabled: true }));
    sheet({ allowed: false });
    expect(screen.getByTestId('leave-self')).toBeTruthy();
    expect(screen.queryByTestId('leave-all')).toBeNull();
  });

  it('"for this phone" asks the host for scope self', async () => {
    const t = transport({ left: true, now_enabled: true });
    setSharingTransport(t);
    sheet();
    fireEvent.click(screen.getByTestId('leave-self'));
    await waitFor(() => expect(screen.getByTestId('leave-done')).toBeTruthy());
    expect(t.calls[0]).toEqual({ hostId: 'peer-1', feature: 'forward', scope: 'self' });
    // Still on for others, because now_enabled is true.
    expect(screen.getByTestId('leave-done').textContent).toContain('Still on for the other phones');
  });

  it('"on the PC" asks the host for scope all, and says so when it ends for everyone', async () => {
    const t = transport({ left: true, now_enabled: false });
    setSharingTransport(t);
    sheet();
    fireEvent.click(screen.getByTestId('leave-all'));
    await waitFor(() => expect(screen.getByTestId('leave-done')).toBeTruthy());
    expect(t.calls[0]).toEqual({ hostId: 'peer-1', feature: 'forward', scope: 'all' });
    expect(screen.getByTestId('leave-done').textContent).toContain('off for everyone now');
  });

  it('a refusal is shown, with the host’s own reason', async () => {
    setSharingTransport(transport({ error: 'web app forwarding is not on for this device' }));
    sheet();
    fireEvent.click(screen.getByTestId('leave-all'));
    await waitFor(() => expect(screen.getByTestId('leave-error')).toBeTruthy());
    expect(screen.getByTestId('leave-error').textContent).toContain('not on for this device');
  });

  it('an older host (proto < 1.15) is told apart from a plain failure', async () => {
    setSharingTransport(transport({ error: 'unknown op feature_leave' }));
    sheet();
    fireEvent.click(screen.getByTestId('leave-self'));
    await waitFor(() => expect(screen.getByTestId('leave-error')).toBeTruthy());
    expect(screen.getByTestId('leave-error').textContent).toContain('older UniPi');
  });
});

describe('the two confirm rows', () => {
  it('name the feature, and "on the PC" only when allowed', () => {
    const both = leaveScopes('browser', true);
    expect(both.map((r) => r.scope)).toEqual(['self', 'all']);
    expect(both[0]!.label).toContain(FEATURE_LABEL.browser.toLowerCase());
    expect(leaveScopes('push', false)).toHaveLength(1);
    // Neither row claims to turn anything on.
    expect(both[0]!.detail).toContain('Other phones keep');
    expect(both[1]!.detail).toContain('You turn it back on from the PC');
  });

  it('an older host is recognised from its error', () => {
    expect(isLeaveUnsupported({ code: 'E_SCHEMA', message: 'unknown variant `feature_leave`', retryable: false })).toBe(true);
    expect(isLeaveUnsupported({ code: 'E_UNAUTHORIZED', message: 'not on for this device', retryable: false })).toBe(false);
  });
});
