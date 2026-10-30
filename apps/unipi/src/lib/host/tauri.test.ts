import { afterEach, describe, expect, it, vi } from 'vitest';

type Msg = Record<string, unknown>;
const channels: Array<{ onmessage?: (m: Msg) => void }> = [];
const invoke = vi.fn<(cmd: string, args?: Record<string, unknown>) => Promise<unknown>>();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args),
  Channel: class {
    onmessage?: (m: Msg) => void;
    constructor() {
      channels.push(this);
    }
  },
}));

import { channelError, createTauriHost, openArgs } from './tauri';

afterEach(() => {
  invoke.mockReset();
  channels.length = 0;
});

const tick = () => new Promise((r) => setTimeout(r, 0));

describe('tauri HostClient: session streams over chat.rs (WP-A2)', () => {
  it('maps open / new / fork to the chat.rs commands', () => {
    expect(openArgs('h1', { kind: 'open', session: 's1', sinceSeq: 42 })).toEqual({ command: 'session_open', args: { hostId: 'h1', session: 's1', sinceSeq: 42 } });
    expect(openArgs('h1', { kind: 'open', session: 's1' })).toEqual({ command: 'session_open', args: { hostId: 'h1', session: 's1' } });
    expect(openArgs('h1', { kind: 'new', cwd: '/p' })).toEqual({ command: 'session_new', args: { hostId: 'h1', cwd: '/p' } });
    expect(openArgs('h1', { kind: 'fork', session: 's1', fromEntry: 'e9' })).toEqual({ command: 'session_fork', args: { hostId: 'h1', session: 's1', entryId: 'e9' } });
  });

  it('routes channel messages to the sink; cmd uses the stream id; close hangs up once', async () => {
    invoke.mockImplementation(async (cmd) => (cmd === 'session_open' ? 'st-1' : cmd === 'session_cmd' ? { ok: 1 } : null));
    const seen: string[] = [];
    const stream = createTauriHost('h1').openSession({ kind: 'open', session: 's1', sinceSeq: 3 }, {
      snapshot: (s) => seen.push(`snap:${s.seq}`),
      events: (items) => seen.push(`events:${items.length}`),
      closed: (e) => seen.push(`closed:${e?.code ?? ''}`),
    });
    expect(invoke).toHaveBeenCalledWith('session_open', expect.objectContaining({ hostId: 'h1', session: 's1', sinceSeq: 3, onEvent: channels[0] }));
    const ch = channels[0]!;
    ch.onmessage!({ kind: 'snapshot', snapshot: { session_id: 's1', reset: true, seq: 3 } });
    ch.onmessage!({ kind: 'events', items: [{ session: 's1', seq: 4, event: { type: 'agent_start' } }] });
    expect(await stream.cmd({ type: 'get_state' })).toEqual({ ok: 1 });
    expect(invoke).toHaveBeenLastCalledWith('session_cmd', expect.objectContaining({ streamId: 'st-1', cmd: { type: 'get_state' }, id: expect.any(Number) }));
    ch.onmessage!({ kind: 'error', message: 'The host went away', code: 'E_TRANSPORT' });
    ch.onmessage!({ kind: 'events', items: [] }); // after the end: ignored
    expect(seen).toEqual(['snap:3', 'events:1', 'closed:E_TRANSPORT']);
    stream.close();
    await tick();
    expect(invoke).toHaveBeenLastCalledWith('session_stream_close', { streamId: 'st-1' });
  });

  it('a failed open closes the sink; close before the id arrives hangs up once it does', async () => {
    let resolve!: (id: string) => void;
    invoke.mockImplementationOnce(() => new Promise((r) => (resolve = r as (id: string) => void)));
    const host = createTauriHost('h1');
    const stream = host.openSession({ kind: 'open', session: 's1' }, { snapshot() {}, events() {}, closed() {} });
    stream.close();
    resolve('st-9');
    await tick();
    expect(invoke).toHaveBeenLastCalledWith('session_stream_close', { streamId: 'st-9' });

    invoke.mockRejectedValueOnce({ code: 'E_NOT_FOUND', message: 'No such session', retryable: false });
    let error: { code?: string } | undefined;
    host.openSession({ kind: 'open', session: 'nope' }, { snapshot() {}, events() {}, closed: (e) => (error = e) });
    await tick();
    expect(error?.code).toBe('E_NOT_FOUND');
  });

  it('events_open streams hub events and reports the end', async () => {
    invoke.mockResolvedValue('ev-1');
    const got: string[] = [];
    let ended: unknown = 'no';
    const sub = createTauriHost('h1').openEvents((e) => got.push(e.kind), (e) => (ended = e));
    expect(invoke).toHaveBeenCalledWith('events_open', expect.objectContaining({ hostId: 'h1' }));
    channels[0]!.onmessage!({ kind: 'hub_event', event: { kind: 'session_removed', session: 's1' } });
    channels[0]!.onmessage!({ kind: 'closed' });
    expect(got).toEqual(['session_removed']);
    expect(ended).toBeUndefined();
    await tick();
    sub.close();
    await tick();
    expect(invoke).toHaveBeenLastCalledWith('events_close', { streamId: 'ev-1' });
  });

  it('channel errors keep transport trouble retryable', () => {
    expect(channelError({ message: 'x' })).toMatchObject({ code: 'E_TRANSPORT', retryable: true });
    expect(channelError({ message: 'x', code: 'E_NOT_FOUND' })).toMatchObject({ code: 'E_NOT_FOUND', retryable: false });
  });
});
