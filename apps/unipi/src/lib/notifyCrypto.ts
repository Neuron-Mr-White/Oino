/**
 * UNI-161 §1: WebCrypto decrypt for the `unipi1:` AES-256-GCM envelope
 * ntfy carries once a per-pairing key is configured (Settings → "Get
 * from this PC" or pasted manually). Mirrors the pi-side encrypt
 * (`packages/notify/ntfy-crypto.ts` in the unipi repo) and the Kotlin
 * `javax.crypto` decrypt used for background delivery (NotifyCrypto.kt) —
 * see `docs/m6/PROTOCOL.md` for the exact byte layout.
 *
 * Format: `unipi1:` + base64(iv(12) || ciphertext || tag(16)).
 */

export const ENVELOPE_PREFIX = 'unipi1:';

export interface NotifyPayload {
  title: string;
  body: string;
  kind?: string;
  host?: string;
  pid?: number;
  session?: string;
  dialogId?: number;
  options?: string[];
}

function base64ToBytes(b64: string): Uint8Array<ArrayBuffer> {
  const binary = atob(b64);
  const buffer = new ArrayBuffer(binary.length);
  const bytes = new Uint8Array(buffer);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

export function base64ToKeyBytes(b64: string): Uint8Array<ArrayBuffer> {
  return base64ToBytes(b64);
}

/** True when `message` is one of our encrypted envelopes (vs. a plain
 *  ntfy message from before a key was configured, or a third party). */
export function isEncryptedEnvelope(message: string): boolean {
  return message.startsWith(ENVELOPE_PREFIX);
}

/** Decrypts one `unipi1:...` envelope with WebCrypto. Throws on a bad
 *  prefix, length, key or auth tag — callers should treat any rejection
 *  as "drop this notification", never show raw ciphertext. */
export async function decryptNotifyEnvelope(keyB64: string, envelope: string): Promise<NotifyPayload> {
  if (!isEncryptedEnvelope(envelope)) throw new Error('missing unipi1: prefix');
  const combined = base64ToBytes(envelope.slice(ENVELOPE_PREFIX.length));
  if (combined.length < 12 + 16) throw new Error('envelope too short');
  const iv = combined.slice(0, 12);
  // WebCrypto AES-GCM wants ciphertext+tag concatenated (not split) —
  // conveniently that's already the layout after the IV.
  const ciphertextAndTag = combined.slice(12);
  const keyBytes = base64ToKeyBytes(keyB64);
  if (keyBytes.length !== 32) throw new Error('notify key must be 32 bytes');
  const cryptoKey = await crypto.subtle.importKey('raw', keyBytes, { name: 'AES-GCM' }, false, ['decrypt']);
  const plaintext = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, cryptoKey, ciphertextAndTag);
  const text = new TextDecoder().decode(plaintext);
  return JSON.parse(text) as NotifyPayload;
}
