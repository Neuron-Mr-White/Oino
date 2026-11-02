/**
 * `startSession` (Z-19): a `session_new` stream in the chosen folder, the
 * draft's choices replayed in order, then the prompt (with images) — and the
 * route waits for the prompt's user message (the session's file exists only
 * from then on, and the chat opens the session by its file).
 */
import { describe, expect, it, vi } from 'vitest';
import type { HostClient, OpenSessionRequest, RpcCommand, SessionSink } from '../../lib/host/client';
import { startSession } from './startSession';

function fakeClient(opts: { fail?: boolean } = {}) {
  const cmds: RpcCommand[] = [];
  let sink: SessionSink | undefined;
  let req: OpenSessionRequest | undefined;
  const close = vi.fn();
  const client = {
    openSession(r: OpenSessionRequest, s: SessionSink) {
      req = r;
      sink = s;
      queueMicrotask(() => (opts.fail ? s.closed({ code: 'E_FAILED', message: 'no room', retryable: false } as never) : s.snapshot({ session_id: 'new-1', reset: true, seq: 0 } as never)));
      return {
        async cmd(c: RpcCommand) {
          cmds.push(c);
          if (c.type === 'prompt' && !String(c.message).startsWith('/')) {
            // The host appends the user message a moment after the ack.
            setTimeout(() => sink!.events([{ session: 'new-1', seq: 5, event: { type: 'message_end', message: { role: 'user' } } } as never]), 30);
          }
          return { disposition: 'started' };
        },
        close,
      };
    },
  } as unknown as HostClient;
  return { client, cmds, close, req: () => req };
}

describe('startSession', () => {
  it('replays the choices, sends the prompt with images and waits for its user message', async () => {
    const f = fakeClient();
    const started = Date.now();
    const res = await startSession(
      'h',
      {
        cwd: '/home/you/p',
        text: 'Fix it',
        images: [{ mimeType: 'image/png', data: 'AAAA' }],
        choices: { model: { provider: 'oino', modelId: 'm2' }, thinking: 'high', permission: 'full', plan: true },
      },
      f.client,
    );
    expect(res).toEqual({ ok: true, sessionId: 'new-1' });
    expect(Date.now() - started).toBeGreaterThanOrEqual(25);
    expect(f.req()).toEqual({ kind: 'new', cwd: '/home/you/p' });
    expect(f.cmds).toEqual([
      { type: 'set_model', provider: 'oino', modelId: 'm2' },
      { type: 'set_thinking_level', level: 'high' },
      { type: 'prompt', message: '/permission full' },
      { type: 'prompt', message: '/plan on' },
      { type: 'prompt', message: 'Fix it', images: [{ type: 'image', data: 'AAAA', mimeType: 'image/png' }] },
    ]);
  });

  it('applies the draft\'s Fusion pair right after the chat starts, through unipi.fusion (no set_model / thinking first)', async () => {
    const f = fakeClient();
    const fusion = { lead: 'anthropic/opus', sidekick: 'zai/glm', leadEffort: 'high', sidekickEffort: 'low' };
    const res = await startSession(
      'h',
      { text: 'Go', images: [], choices: { model: { provider: 'anthropic', modelId: 'opus' }, thinking: 'high', fusion, permission: 'ask' } },
      f.client,
    );
    expect(res).toEqual({ ok: true, sessionId: 'new-1' });
    expect(f.cmds).toEqual([
      { type: '_oino/service_call', name: 'fusion.set-pair', payload: { active: { kind: 'fusion', ...fusion } } },
      { type: 'prompt', message: '/permission ask' },
      { type: 'prompt', message: 'Go' },
    ]);
  });

  it('a stream that closes before its snapshot is an error', async () => {
    const f = fakeClient({ fail: true });
    const res = await startSession('h', { text: 'x', images: [], choices: {} }, f.client);
    expect(res.ok).toBe(false);
    expect(f.close).toHaveBeenCalled();
  });
});
