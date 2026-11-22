import { beforeEach, describe, expect, it } from 'vitest';
import type { SessionHandle } from '../session/store';
import { setChatModel } from './chatModel';
import { activeSessionFusion, fetchFusion, resetFusionOverrides, setFusionActive } from './fusion';

beforeEach(resetFusionOverrides);
const pair = { kind: 'fusion' as const, lead: 'mock/one', sidekick: 'mock/two' };
const status = { leadName: 'One', sidekickName: 'Two', leadKey: pair.lead, sidekickKey: pair.sidekick };

describe('MC-F1 authoritative model refresh', () => {
  it.each([false, true])('single refreshes after a command (failure=%s)', async (fail) => {
    let refreshed = 0;
    const s = { cmd: async () => { if (fail) throw new Error('observer'); }, refreshModel: async () => { refreshed++; } } as unknown as SessionHandle;
    await setChatModel(s, 'mock', 'two').catch(() => {});
    expect(refreshed).toBe(1);
  });

  it.each([false, true])('Fusion refreshes host truth and clears an old sticky pair (failure=%s)', async (fail) => {
    let active: unknown = pair;
    let refreshed = 0;
    const s = {
      key: 'h/s', state: { model: { provider: 'mock', id: 'one' }, topics: { 'unipi:fusion:status': { data: status } }, entries: [] },
      cmd: async (cmd: { name: string }) => {
        if (cmd.name === 'fusion.status') return { active, ...(active === pair ? { status } : {}) };
        active = { kind: 'single', model: 'mock/one' };
        if (fail) throw new Error('observer failed after commit');
        return { ok: true };
      },
      refreshModel: async () => { refreshed++; },
    } as unknown as SessionHandle;
    await fetchFusion(s);
    expect(activeSessionFusion(s)?.sidekick).toBe(pair.sidekick);
    await setFusionActive(s, { kind: 'single', model: 'mock/one' }).catch(() => {});
    expect(refreshed).toBe(1);
    expect(activeSessionFusion(s)).toBeUndefined();
    // A newer bus publication can enable Fusion again after the point refresh.
    s.state.topics['unipi:fusion:status'] = { data: status, sticky: true, publisher: 'unipi.fusion' };
    expect(activeSessionFusion(s)?.sidekick).toBe(pair.sidekick);
  });
});
