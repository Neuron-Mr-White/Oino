/**
 * UNI-161 §5: ntfy history poll — "Keep listening" fallback when no
 * UnifiedPush distributor is installed. On open/resume, polls
 * `GET {server}/{topic}/json?poll=1&since=…` with the configured token,
 * decrypts any `unipi1:` message with the stored key (notifyCrypto.ts),
 * and feeds decrypted items into the "needs you" drawer (lib/needsYou.ts).
 * Messages that aren't encrypted (no key configured, or a plain ntfy
 * message from before pairing) are ignored here — they still show as a
 * native OS notification via the browser/service worker path, just not
 * in the drawer (no structured host/pid/dialog to route with).
 */
import { decryptNotifyEnvelope, isEncryptedEnvelope } from './notifyCrypto';
import { needsYou, needsYouId, groupFor, type NeedsYouItem } from './needsYou';

export interface NtfyHistoryEntry {
  id: string;
  time: number;
  event: string;
  title?: string;
  message?: string;
}

/** Parses one ntfy `/json?poll=1` response body (newline-delimited JSON). */
export function parseNtfyHistory(raw: string): NtfyHistoryEntry[] {
  const out: NtfyHistoryEntry[] = [];
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    try {
      const parsed = JSON.parse(trimmed) as Partial<NtfyHistoryEntry>;
      if (parsed.event === 'message' && typeof parsed.id === 'string' && typeof parsed.time === 'number') {
        out.push({ id: parsed.id, time: parsed.time, event: parsed.event, title: parsed.title, message: parsed.message });
      }
    } catch {
      // one malformed line never breaks the rest of the poll
    }
  }
  return out;
}

/** Polls the configured ntfy topic's recent history, decrypts any
 *  `unipi1:` messages with `keyB64`, and upserts them into the "needs
 *  you" drawer. Best-effort: network/parse/decrypt failures are swallowed
 *  — this is a fallback poll, never something that should surface an
 *  error toast on every resume. */
export async function pollNtfyHistory(
  serverUrl: string,
  topic: string,
  token: string | undefined,
  keyB64: string | undefined,
  sinceSeconds: number = 600,
): Promise<void> {
  if (!serverUrl || !topic) return;
  try {
    const base = serverUrl.replace(/\/$/, '');
    const url = `${base}/${topic}/json?poll=1&since=${sinceSeconds}s`;
    const headers: Record<string, string> = {};
    if (token) headers.Authorization = `Bearer ${token}`;
    const res = await fetch(url, { headers });
    if (!res.ok) return;
    const text = await res.text();
    const entries = parseNtfyHistory(text);
    const store = needsYou();
    for (const entry of entries) {
      if (!entry.message || !keyB64 || !isEncryptedEnvelope(entry.message)) continue;
      try {
        const payload = await decryptNotifyEnvelope(keyB64, entry.message);
        const kind = payload.kind ?? 'other';
        const since = entry.time * 1000;
        const item: NeedsYouItem = {
          id: needsYouId(payload.host ?? 'unknown', payload.pid ?? null, kind, since),
          hostId: payload.host ?? 'unknown',
          hostName: payload.host ?? 'Unknown host',
          pid: payload.pid ?? null,
          session: payload.session ?? '',
          title: payload.title,
          kind,
          group: groupFor(kind),
          what: payload.body,
          since,
        };
        store.upsert(item);
      } catch {
        // bad tag/wrong key/corrupt envelope: drop silently, never show ciphertext
      }
    }
  } catch {
    // network error: the next resume/poll tries again
  }
}
