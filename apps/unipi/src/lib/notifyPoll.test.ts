// @vitest-environment jsdom
/**
 * UNI-161 §5: ntfy history poll decrypts `unipi1:` entries and feeds the
 * "needs you" drawer; plaintext/undecryptable entries never surface
 * their raw content.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { parseNtfyHistory, pollNtfyHistory } from './notifyPoll';
import { needsYou } from './needsYou';

function freshNeedsYou() {
  const store = needsYou();
  store.clearAll();
  return store;
}

describe('parseNtfyHistory', () => {
  it('parses newline-delimited ntfy message events, skipping non-message lines', () => {
    const raw = [
      JSON.stringify({ id: 'a1', time: 1000, event: 'open' }),
      JSON.stringify({ id: 'a2', time: 1001, event: 'message', title: 'UniPi', message: 'unipi1:abc' }),
      '',
      'not json',
      JSON.stringify({ id: 'a3', time: 1002, event: 'message', title: 'UniPi', message: 'unipi1:def' }),
    ].join('\n');
    const entries = parseNtfyHistory(raw);
    expect(entries.map((e) => e.id)).toEqual(['a2', 'a3']);
  });

  it('returns [] for an empty or garbage body', () => {
    expect(parseNtfyHistory('')).toEqual([]);
    expect(parseNtfyHistory('garbage\nmore garbage')).toEqual([]);
  });
});

describe('pollNtfyHistory', () => {
  let originalFetch: typeof fetch;
  beforeEach(() => {
    originalFetch = global.fetch;
  });
  afterEach(() => {
    global.fetch = originalFetch;
    localStorage.clear();
  });

  it('decrypts an encrypted entry and upserts it into the needs-you drawer', async () => {
    const store = freshNeedsYou();
    const keyBytes = crypto.getRandomValues(new Uint8Array(32));
    const keyB64 = Buffer.from(keyBytes).toString('base64');
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const payload = { title: 'Pi asks', body: 'Which plan?', kind: 'ask_user', host: 'pc1', pid: 7 };
    const cryptoKey = await crypto.subtle.importKey('raw', keyBytes, { name: 'AES-GCM' }, false, ['encrypt']);
    const plaintext = new TextEncoder().encode(JSON.stringify(payload));
    const ciphertext = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, cryptoKey, plaintext);
    const combined = new Uint8Array(iv.length + ciphertext.byteLength);
    combined.set(iv, 0);
    combined.set(new Uint8Array(ciphertext), iv.length);
    const envelope = `unipi1:${Buffer.from(combined).toString('base64')}`;

    const body = [JSON.stringify({ id: 'n1', time: 1700000000, event: 'message', title: 'UniPi', message: envelope })].join('\n');
    global.fetch = vi.fn(async () => new Response(body, { status: 200 })) as unknown as typeof fetch;

    await pollNtfyHistory('https://ntfy.example', 'my-topic', 'tk_x', keyB64);

    const items = store.list();
    expect(items).toHaveLength(1);
    expect(items[0]!.title).toBe('Pi asks');
    expect(items[0]!.what).toBe('Which plan?');
    expect(items[0]!.hostId).toBe('pc1');
    expect(items[0]!.pid).toBe(7);
    expect(items[0]!.kind).toBe('ask_user');
  });

  it('silently drops an entry that fails to decrypt (wrong key)', async () => {
    const store = freshNeedsYou();
    const body = [JSON.stringify({ id: 'n1', time: 1700000000, event: 'message', title: 'UniPi', message: 'unipi1:bm90LWEtcmVhbC1lbnZlbG9wZQ==' })].join('\n');
    global.fetch = vi.fn(async () => new Response(body, { status: 200 })) as unknown as typeof fetch;

    const wrongKey = Buffer.alloc(32, 1).toString('base64');
    await pollNtfyHistory('https://ntfy.example', 'my-topic', 'tk_x', wrongKey);

    expect(store.list()).toHaveLength(0);
  });

  it('ignores plaintext (unencrypted) messages — never shown as drawer content', async () => {
    const store = freshNeedsYou();
    const body = [JSON.stringify({ id: 'n1', time: 1700000000, event: 'message', title: 'UniPi', message: 'a plain legacy message' })].join('\n');
    global.fetch = vi.fn(async () => new Response(body, { status: 200 })) as unknown as typeof fetch;

    await pollNtfyHistory('https://ntfy.example', 'my-topic', 'tk_x', Buffer.alloc(32, 2).toString('base64'));

    expect(store.list()).toHaveLength(0);
  });

  it('does nothing without a configured server/topic', async () => {
    global.fetch = vi.fn() as unknown as typeof fetch;
    await pollNtfyHistory('', '', undefined, undefined);
    expect(global.fetch).not.toHaveBeenCalled();
  });

  it('swallows network errors', async () => {
    global.fetch = vi.fn(async () => {
      throw new Error('network down');
    }) as unknown as typeof fetch;
    await expect(pollNtfyHistory('https://ntfy.example', 't', undefined, undefined)).resolves.toBeUndefined();
  });
});
