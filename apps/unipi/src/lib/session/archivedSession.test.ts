import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';

const fsRead = vi.fn();
vi.mock('../fs', () => ({ fsRead: (...a: unknown[]) => fsRead(...a) }));

import { createArchivedSession } from './archivedSession';

const card = { session_id: 'old1', file: '/trash/old1.jsonl', cwd: '/p', name: 'Old chat' } as never;
const tick = () => new Promise((r) => setTimeout(r, 0));
const jsonl = [
  JSON.stringify({ type: 'session', id: 'old1', cwd: '/p' }),
  JSON.stringify({ type: 'message', id: 'u1', parentId: null, message: { role: 'user', content: 'hi' } }),
  JSON.stringify({ type: 'message', id: 'a1', parentId: 'u1', message: { role: 'assistant', content: [{ type: 'text', text: 'hello' }] } }),
].join('\n');

describe('archived chat handle (chat-vanish: never the empty "new chat" for a chat with a transcript)', () => {
  afterEach(() => fsRead.mockReset());

  it('a failed read says so (closed + error → the timeline error card), and Retry reads again', async () => {
    fsRead.mockResolvedValueOnce({ ok: false, error: { code: 'E_TOO_LARGE', message: 'too big' } });
    const h = createRoot(() => createArchivedSession('h1', card));
    expect(h.state.conn).toBe('connecting');
    await tick();
    await tick();
    // Before: conn stayed 'live' with 0 entries — the pane showed "Say hi", and Retry was a no-op.
    expect(h.state.conn).toBe('closed');
    expect(h.state.connError?.message).toBe('too big');
    fsRead.mockResolvedValueOnce({ ok: true, value: { text: jsonl } });
    h.retry();
    await tick();
    await tick();
    expect(h.state.conn).toBe('live');
    expect(h.state.entries.map((e) => e.id)).toEqual(['u1', 'a1']);
    expect(fsRead).toHaveBeenCalledTimes(2);
  });

  it('a closed handle ignores a late read', async () => {
    let resolve!: (v: unknown) => void;
    fsRead.mockReturnValueOnce(new Promise((r) => (resolve = r)));
    const h = createRoot(() => createArchivedSession('h1', card));
    h.close();
    resolve({ ok: true, value: { text: jsonl } });
    await tick();
    expect(h.state.entries).toHaveLength(0);
  });
});
