/**
 * Render cache (WP-A8 acceptance): keyed `(target, key, entryId, rev,
 * width)`; in-flight requests dedupe; a 2 s timeout (the stream's
 * `E_TIMEOUT` rejection) resolves `null` so **nothing renders** and the
 * frontend's own row stays; a `rev`/width change re-asks.
 */
import { createRoot } from 'solid-js';
import { describe, expect, it, vi } from 'vitest';
import { createRenderCache, widthClassOf, type RenderRowKey } from './renderCache';
import type { SessionHandle } from './store';

type Reply = { node?: unknown } | Promise<never>;

const fakeSession = (reply: (row: RenderRowKey) => Reply) =>
  ({
    key: 'h1/s1',
    cmd: vi.fn((cmd: { key?: unknown; target?: unknown; entryId?: unknown; width?: unknown; rev?: unknown }) => {
      const row = {
        target: cmd.target,
        key: String(cmd.key ?? ''),
        entryId: cmd.entryId === undefined ? undefined : String(cmd.entryId),
        rev: Number(cmd.rev ?? 0),
        width: cmd.width,
      } as RenderRowKey;
      return Promise.resolve(reply(row));
    }),
  }) as unknown as SessionHandle;

const row = (over: Partial<RenderRowKey> = {}): RenderRowKey => ({ target: 'customMessage', key: 'unipi:watchdog', entryId: 'e1', rev: 0, width: 'normal', ...over });

describe('widthClassOf', () => {
  it('maps px to the narrow/normal/wide classes', () => {
    expect(widthClassOf(480)).toBe('narrow');
    expect(widthClassOf(720)).toBe('normal');
    expect(widthClassOf(1000)).toBe('wide');
  });
});

describe('renderCache', () => {
  it('caches by (target, key, entryId, rev, width): one command per key', async () => {
    const node = { t: 'Paragraph', text: 'hi' };
    const session = fakeSession(() => ({ node }));
    await createRoot(async (dispose) => {
      const cache = createRenderCache(session);
      const a = cache.node(row(), { data: {} });
      const b = cache.node(row(), { data: {} }); // same key: deduped
      expect(a()).toBeUndefined(); // in flight → render nothing yet
      await vi.waitFor(() => expect(a()).toEqual(node));
      expect(b()).toEqual(node);
      expect((session.cmd as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(1);
      // a different entry/rev/width is a different key
      cache.node(row({ entryId: 'e2' }), { data: {} });
      cache.node(row({ rev: 1 }), { data: {} });
      cache.node(row({ width: 'wide' }), { data: {} });
      await vi.waitFor(() => expect(cache.size).toBe(4));
      expect((session.cmd as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(4);
      dispose();
    });
  });

  it('a 2 s timeout (rejection) resolves null — nothing renders', async () => {
    const session = fakeSession(() => Promise.reject({ code: 'E_TIMEOUT', message: 'The session did not answer in time' }));
    await createRoot(async (dispose) => {
      const cache = createRenderCache(session);
      const a = cache.node(row(), { data: {} });
      await vi.waitFor(() => expect(a()).toBeNull());
      // and the null result is remembered: no second command
      const b = cache.node(row(), { data: {} });
      expect(b()).toBeNull();
      expect((session.cmd as ReturnType<typeof vi.fn>).mock.calls).toHaveLength(1);
      dispose();
    });
  });

  it('`node: null` (no renderer registered) is the frontend-own-rendering answer', async () => {
    const session = fakeSession(() => ({ node: null }));
    await createRoot(async (dispose) => {
      const cache = createRenderCache(session);
      const a = cache.node(row(), { data: {} });
      await vi.waitFor(() => expect(a()).toBeNull());
      dispose();
    });
  });

  it('a rev bump re-asks (the invalidate path)', async () => {
    let n = 0;
    const session = fakeSession(() => ({ node: { t: 'Paragraph', text: `v${++n}` } }));
    await createRoot(async (dispose) => {
      const cache = createRenderCache(session);
      const v0 = cache.node(row({ rev: 0 }), { data: {} });
      await vi.waitFor(() => expect(v0()).toEqual({ t: 'Paragraph', text: 'v1' }));
      const v1 = cache.node(row({ rev: 1 }), { data: {} });
      await vi.waitFor(() => expect(v1()).toEqual({ t: 'Paragraph', text: 'v2' }));
      dispose();
    });
  });
});
