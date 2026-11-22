/**
 * `startSession` (Z-19): a `session_new` stream in the chosen folder, the
 * draft's choices replayed in order, then the prompt (with images) — and the
 * route waits for the prompt's user message (the session's file exists only
 * from then on, and the chat opens the session by its file).
 */
import { describe, expect, it, vi } from 'vitest';
import type { HostClient, OpenSessionRequest, RpcCommand, SessionSink } from '../../lib/host/client';
import { setBlobTransport } from '../../lib/blob';
import { startSession } from './startSession';
import { toasts } from '../../lib/toasts';

function fakeClient(opts: { fail?: boolean; refusal?: string; notice?: string } = {}) {
  const cmds: RpcCommand[] = [];
  let sink: SessionSink | undefined;
  let req: OpenSessionRequest | undefined;
  const close = vi.fn();
  const requests: Array<[string, unknown]> = [];
  const client = {
    async request(op: string, args?: unknown) {
      requests.push([op, args]);
      return {};
    },
    openSession(r: OpenSessionRequest, s: SessionSink) {
      req = r;
      sink = s;
      queueMicrotask(() => (opts.fail ? s.closed({ code: 'E_FAILED', message: 'no room', retryable: false } as never) : s.snapshot({ session_id: 'new-1', reset: true, seq: 0 } as never)));
      return {
        async cmd(c: RpcCommand) {
          cmds.push(c);
          if (opts.notice && c.type === 'prompt') {
            sink!.events([{ session: 'new-1', seq: 1, event: {
              type: 'extension_ui_request', id: 'orphan-notice', method: 'notify',
              notifyType: 'info', message: opts.notice,
            } } as never]);
          }
          if (c.type === 'prompt' && !String(c.message).startsWith('/')) {
            // The host appends the user message a moment after the ack.
            setTimeout(() => sink!.events([{ session: 'new-1', seq: 5, event: { type: 'message_end', message: { role: 'user' } } } as never]), 30);
          }
          return opts.refusal ? { disposition: 'refused', reason: opts.refusal } : { disposition: 'started' };
        },
        close,
      };
    },
  } as unknown as HostClient;
  return { client, cmds, close, requests, req: () => req };
}

describe('startSession', () => {
  it('shows a real-host-shaped orphan notice received before the chat mounts', async () => {
    const notice = 'Closed a graph left open by a deleted chat (Seed graph task). Starting your swarm.';
    const f = fakeClient({ notice });
    const res = await startSession('h', { text: 'Start the swarm', images: [], choices: {} }, f.client);
    expect(res.ok).toBe(true);
    expect(toasts().list().some((toast) => toast.message === notice)).toBe(true);
  });

  it('a refused first /swarm is a failed start so the draft restores text and images', async () => {
    const f = fakeClient({ refusal: 'Another chat is running a graph' });
    const images = [{ mimeType: 'image/png', data: 'AAAA' }, { mimeType: 'image/png', data: 'BBBB' }];
    const res = await startSession('h', { text: '/swarm keep the images', images, choices: {} }, f.client);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.message).toBe('Another chat is running a graph');
    expect(f.cmds[0]).toEqual({ type: 'prompt', message: '/swarm keep the images',
      images: images.map((image) => ({ type: 'image', ...image })) });
    expect(f.close).toHaveBeenCalledOnce();
    // D3: the empty chat the refused start created is archived and deleted, not left in the lists.
    expect(f.requests).toEqual([['session_archive', { session: 'new-1' }], ['session_delete', { session: 'new-1' }]]);
  });

  it('N8: a refusal is shown once, inline: the command\'s own notice that repeats the reason is not toasted', async () => {
    const reason = 'Swarm not started: The chat “Live graph owner chat” is running the graph “ZMS-1”';
    const f = fakeClient({ refusal: reason, notice: reason });
    const res = await startSession('h', { text: '/swarm go', images: [], choices: {} }, f.client);
    expect(res.ok).toBe(false);
    if (!res.ok) expect(res.error.message).toBe(reason);
    expect(toasts().list().some((toast) => toast.message === reason)).toBe(false);
  });

  it('N8: a different notice that came with a refusal is still shown', async () => {
    const other = 'Closed a graph left open by a deleted chat (Seed). Starting your swarm.';
    const f = fakeClient({ refusal: 'Swarm not started: busy', notice: other });
    await startSession('h', { text: '/swarm go', images: [], choices: {} }, f.client);
    expect(toasts().list().some((toast) => toast.message === other)).toBe(true);
  });

  it('a start that fails before the host answers has no chat to remove', async () => {
    const f = fakeClient({ fail: true });
    const res = await startSession('h', { text: 'hi', images: [], choices: {} }, f.client);
    expect(res.ok).toBe(false);
    expect(f.requests).toEqual([]);
  });

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

  it('big pasted images go as blob references (a chat\'s rule), not inlined into the prompt frame', async () => {
    const f = fakeClient();
    const uploaded: Array<{ host: string; size: number; mime: string }> = [];
    setBlobTransport({
      async putBytes(host, bytes, mime) {
        uploaded.push({ host, size: bytes.length, mime });
        return { sha256: 'f'.repeat(64), size: bytes.length, path: '/blob', deduped: false };
      },
      async putFile() { throw new Error('unused'); },
      async getFile() { throw new Error('unused'); },
      async getBytes() { throw new Error('unused'); },
    });
    try {
      const big = btoa('x'.repeat(600 * 1024));
      const res = await startSession('h', { text: 'look', images: [{ mimeType: 'image/png', data: big }, { mimeType: 'image/png', data: big }], choices: {} }, f.client);
      expect(res.ok).toBe(true);
      expect(uploaded).toEqual([{ host: 'h', size: 600 * 1024, mime: 'image/png' }, { host: 'h', size: 600 * 1024, mime: 'image/png' }]);
      expect(f.cmds.at(-1)).toEqual({
        type: 'prompt',
        message: 'look',
        images: [{ type: 'image', mediaRef: 'f'.repeat(64), mimeType: 'image/png' }, { type: 'image', mediaRef: 'f'.repeat(64), mimeType: 'image/png' }],
      });
    } finally {
      setBlobTransport(undefined);
    }
  });

  it('a stream that closes before its snapshot is an error', async () => {
    const f = fakeClient({ fail: true });
    const res = await startSession('h', { text: 'x', images: [], choices: {} }, f.client);
    expect(res.ok).toBe(false);
    expect(f.close).toHaveBeenCalled();
  });
});
