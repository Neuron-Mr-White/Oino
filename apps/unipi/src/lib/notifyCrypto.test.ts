import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { decryptNotifyEnvelope, isEncryptedEnvelope } from './notifyCrypto';

const fixture = JSON.parse(readFileSync('./protocol/fixtures/notify-crypto.json', 'utf8')) as {
  keyB64: string;
  envelope: string;
  payload: Record<string, unknown>;
};

describe('decryptNotifyEnvelope', () => {
  it('decrypts the cross-runtime fixture envelope (encrypted on the pi side with node:crypto)', async () => {
    const payload = await decryptNotifyEnvelope(fixture.keyB64, fixture.envelope);
    expect(payload).toEqual(fixture.payload);
  });

  it('recognizes an encrypted envelope by its unipi1: prefix', () => {
    expect(isEncryptedEnvelope(fixture.envelope)).toBe(true);
    expect(isEncryptedEnvelope('plain ntfy message')).toBe(false);
  });

  it('round-trips a freshly encrypted payload (WebCrypto encrypt + decrypt, no fixed IV)', async () => {
    const keyBytes = crypto.getRandomValues(new Uint8Array(32));
    const keyB64 = Buffer.from(keyBytes).toString('base64');
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const payload = { title: 'Hi', body: 'There', kind: 'ask_user' };
    const cryptoKey = await crypto.subtle.importKey('raw', keyBytes, { name: 'AES-GCM' }, false, ['encrypt']);
    const plaintext = new TextEncoder().encode(JSON.stringify(payload));
    const ciphertext = await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, cryptoKey, plaintext);
    const combined = new Uint8Array(iv.length + ciphertext.byteLength);
    combined.set(iv, 0);
    combined.set(new Uint8Array(ciphertext), iv.length);
    const envelope = `unipi1:${Buffer.from(combined).toString('base64')}`;
    const decrypted = await decryptNotifyEnvelope(keyB64, envelope);
    expect(decrypted).toEqual(payload);
  });

  it('rejects a message with no unipi1: prefix', async () => {
    await expect(decryptNotifyEnvelope(fixture.keyB64, 'not-encrypted')).rejects.toThrow();
  });

  it('rejects the wrong key', async () => {
    const wrongKey = Buffer.alloc(32, 9).toString('base64');
    await expect(decryptNotifyEnvelope(wrongKey, fixture.envelope)).rejects.toThrow();
  });
});
