import { createRoot, createSignal } from 'solid-js';
import { afterEach, expect, it, vi } from 'vitest';
import { createSuperagentState } from './state';
import { FleetSchema, StatusSchema, superagentApi } from './api';

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

it('never overlaps polls and keeps polling past a hung fleet call (bounded work)', async () => {
  vi.useFakeTimers();
  let inFlight = 0; let maxInFlight = 0;
  // The fleet call never answers: each poll waits for the 12 s stall timeout, then the next may start.
  vi.spyOn(superagentApi, 'fleet').mockImplementation(() => new Promise(() => {}));
  const status = vi.spyOn(superagentApi, 'status').mockImplementation(async () => {
    inFlight++; maxInFlight = Math.max(maxInFlight, inFlight);
    await Promise.resolve();
    inFlight--;
    return { ok: true, value: StatusSchema.parse({ enabled: true }) };
  });
  vi.spyOn(superagentApi, 'sessions').mockResolvedValue({ ok: true, value: [] });
  vi.spyOn(superagentApi.presets, 'list').mockResolvedValue({ ok: true, value: { presets: [], defaultPresetId: null } });
  let dispose!: () => void;
  let state!: ReturnType<typeof createSuperagentState>;
  createRoot((d) => { dispose = d; state = createSuperagentState(() => 'host'); });
  await vi.advanceTimersByTimeAsync(100_000);
  expect(maxInFlight).toBe(1);
  // One poll per stall window at most (12 s), never one per timer tick.
  expect(status.mock.calls.length).toBeGreaterThan(1);
  expect(status.mock.calls.length).toBeLessThanOrEqual(10);
  expect(state.fleet().nodes).toHaveLength(0);
  dispose();
});

it('a fleet answer that arrives is applied without growing nodes across polls', async () => {
  vi.useFakeTimers();
  const one = FleetSchema.parse({ nodes: [{ id: 'lead', role: 'lead', status: 'working', projectId: 'p' }] });
  vi.spyOn(superagentApi, 'fleet').mockResolvedValue({ ok: true, value: one });
  vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: StatusSchema.parse({ enabled: true }) });
  vi.spyOn(superagentApi, 'sessions').mockResolvedValue({ ok: true, value: [] });
  vi.spyOn(superagentApi.presets, 'list').mockResolvedValue({ ok: true, value: { presets: [], defaultPresetId: null } });
  let dispose!: () => void;
  let state!: ReturnType<typeof createSuperagentState>;
  createRoot((d) => { dispose = d; state = createSuperagentState(() => 'host'); });
  await vi.advanceTimersByTimeAsync(60_000);
  expect(state.fleet().nodes).toHaveLength(1);
  dispose();
});

const setup = (fleet: () => Promise<any>) => {
  vi.spyOn(superagentApi, 'fleet').mockImplementation(fleet);
  vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: StatusSchema.parse({ enabled: true }) });
  vi.spyOn(superagentApi, 'sessions').mockResolvedValue({ ok: true, value: [] });
  vi.spyOn(superagentApi.presets, 'list').mockResolvedValue({ ok: true, value: { presets: [], defaultPresetId: null } });
  let dispose!: () => void;
  let state!: ReturnType<typeof createSuperagentState>;
  createRoot((d) => { dispose = d; state = createSuperagentState(() => 'host'); });
  return { state, dispose };
};

it('keeps the last graph and counts failed polls; a good poll clears them and stamps updatedAt', async () => {
  vi.useFakeTimers();
  const one = FleetSchema.parse({ nodes: [{ id: 'lead', role: 'lead', status: 'working', projectId: 'p' }], rev: 'a' });
  let fail = false;
  const { state, dispose } = setup(async () => (fail ? { ok: false, error: { code: 'E_INTERNAL', message: 'boom', retryable: false } } : { ok: true, value: one }));
  await vi.advanceTimersByTimeAsync(100);
  expect(state.failedPolls()).toBe(0);
  expect(state.updatedAt()).not.toBeNull();
  const stamped = state.updatedAt();
  fail = true;
  await vi.advanceTimersByTimeAsync(5_000 * 3 + 100);
  expect(state.failedPolls()).toBeGreaterThanOrEqual(3);
  expect(state.fleet().nodes).toHaveLength(1); // the last graph stays
  expect(state.fleetError()?.code).toBe('E_INTERNAL');
  expect(state.updatedAt()).toBe(stamped);
  fail = false;
  await vi.advanceTimersByTimeAsync(5_000);
  expect(state.failedPolls()).toBe(0);
  expect(state.fleetError()).toBeUndefined();
  dispose();
});

it('a hung fleet call counts as a failed poll, never stacks, and leaves updatedAt on the last good answer', async () => {
  vi.useFakeTimers();
  let calls = 0;
  const one = FleetSchema.parse({ nodes: [{ id: 'lead', role: 'lead', status: 'working', projectId: 'p' }] });
  const { state, dispose } = setup(() => { calls++; return calls === 1 ? Promise.resolve({ ok: true, value: one }) : new Promise(() => {}); });
  await vi.advanceTimersByTimeAsync(100);
  const stamped = state.updatedAt();
  await vi.advanceTimersByTimeAsync(40_000);
  expect(calls).toBeLessThanOrEqual(4); // one poll per 12 s stall window, not one per 5 s tick
  expect(state.failedPolls()).toBeGreaterThanOrEqual(2);
  expect(state.updatedAt()).toBe(stamped);
  expect(state.fleet().nodes).toHaveLength(1);
  dispose();
});

it('an older host that does not know superagent.fleet falls back to status + history, and stops asking', async () => {
  vi.useFakeTimers();
  const { state, dispose } = setup(async () => ({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant', retryable: false } }));
  await vi.advanceTimersByTimeAsync(100);
  expect(state.fleetLoaded()).toBe(true);
  expect(state.failedPolls()).toBe(0);
  const calls = (superagentApi.fleet as any).mock.calls.length;
  await vi.advanceTimersByTimeAsync(11_000);
  expect((superagentApi.fleet as any).mock.calls.length).toBe(calls);
  dispose();
});

it('re-issuing the selected host record (same id) does not wipe the fleet or restart polling', async () => {
  vi.useFakeTimers();
  const one = FleetSchema.parse({ nodes: [{ id: 'lead', role: 'lead', status: 'working', projectId: 'p' }] });
  vi.spyOn(superagentApi, 'fleet').mockResolvedValue({ ok: true, value: one });
  vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: StatusSchema.parse({ enabled: true }) });
  vi.spyOn(superagentApi, 'sessions').mockResolvedValue({ ok: true, value: [] });
  vi.spyOn(superagentApi.presets, 'list').mockResolvedValue({ ok: true, value: { presets: [], defaultPresetId: null } });
  let dispose!: () => void;
  let state!: ReturnType<typeof createSuperagentState>;
  const [record, setRecord] = createSignal({ id: 'host', online: true });
  createRoot((d) => { dispose = d; state = createSuperagentState(() => record().id); });
  await vi.advanceTimersByTimeAsync(100);
  expect(state.fleet().nodes).toHaveLength(1);
  const calls = (superagentApi.status as any).mock.calls.length;
  setRecord({ id: 'host', online: true });
  await vi.advanceTimersByTimeAsync(10);
  expect(state.fleetLoaded()).toBe(true);
  expect(state.fleet().nodes).toHaveLength(1);
  expect((superagentApi.status as any).mock.calls.length).toBe(calls);
  dispose();
});
