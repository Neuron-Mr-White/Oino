/**
 * Host-synced drafts (owner 2026-11-06): a chat's composer draft — text, quote and paste chips,
 * and its attachments (images edits included) — is kept by the PC (`draft_set` / `draft_get`), so
 * it survives a restart and shows on every device; the session card's `draft_at` puts "Draft" in
 * the sidebar.
 *
 * - **Save**: throttled per chat (`SAVE_THROTTLE_MS`, first change at once, then at most one write
 *   per window with the latest state). Images go to the PC's blob store first (deduplicated by
 *   sha256, cached per image version); a file attachment is already on the PC (its path).
 * - **Load**: once per chat per app run, when its composer mounts. The PC's draft wins over the
 *   text a reload reads back from local storage (that copy has no attachments); a draft this run
 *   already holds in memory wins over the PC's (it is newer) and is saved up.
 * - **Clear**: a send clears it at once.
 *
 * Only real sessions sync (`<host>/<session>` keys); a new-chat draft stays local.
 */
import { blobTransport } from '../blob';
import { hostClient } from '../host/factory';
import { addReadyFile, addReadyImage, markPlaced, pendingAttachmentsFor, type PendingAttachment } from './attachments';
import { attachmentToken, imageToken } from './chips';
import { applyHostDraft, composerDraft, hasLiveDraft, type ComposerChip, type QuoteChip } from './drafts';

export const SAVE_THROTTLE_MS = 800;

type DraftAttachment = { id: string; kind: 'image' | 'file'; name: string; mime: string; size: number; sha256?: string; path?: string };
export type HostDraft = { v: 1; text: string; quotes: QuoteChip[]; chips: ComposerChip[]; attachments: DraftAttachment[] };

/** `<host>/<session>` → its parts; `undefined` for a new-chat draft key. */
export function sessionOfKey(key: string): { hostId: string; sessionId: string } | undefined {
  if (key.startsWith('draft:')) return undefined;
  const at = key.indexOf('/');
  if (at <= 0 || at === key.length - 1) return undefined;
  return { hostId: key.slice(0, at), sessionId: key.slice(at + 1) };
}

const timers = new Map<string, ReturnType<typeof setTimeout>>();
const lastSave = new Map<string, number>();
/** Uploaded image versions: `<id>:<size>:<data length>` → sha256. */
const uploaded = new Map<string, string>();
const loaded = new Set<string>();
/** The last document sent per chat (skip identical writes). */
const sent = new Map<string, string>();

const base64Bytes = (b64: string) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
function bytesBase64(bytes: Uint8Array): string {
  let bin = '';
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}

async function attachmentDoc(hostId: string, a: PendingAttachment): Promise<DraftAttachment | undefined> {
  if (a.status !== 'ready') return undefined;
  const base = { id: a.id, kind: a.kind, name: a.name, mime: a.mime, size: a.size };
  if (a.kind === 'file') return a.path ? { ...base, path: a.path } : undefined;
  if (!a.data) return undefined;
  const version = `${a.id}:${a.size}:${a.data.length}`;
  let sha = uploaded.get(version);
  if (!sha) {
    sha = (await blobTransport().putBytes(hostId, base64Bytes(a.data), a.mime, a.name)).sha256;
    uploaded.set(version, sha);
  }
  return { ...base, sha256: sha };
}

/** The chat's draft as the PC keeps it (`null` = nothing worth keeping). */
export async function buildDraft(key: string, hostId: string): Promise<HostDraft | null> {
  const d = composerDraft(key);
  const referenced = new Set([...d.text.matchAll(/@(?:img|att):(\S+)/g)].map((m) => m[1]!));
  const attachments: DraftAttachment[] = [];
  for (const a of pendingAttachmentsFor(key)) {
    if (a.placed && !referenced.has(a.id)) continue; // a deleted chip (kept only for Undo)
    // An image the PC can't take right now (no blob transport, offline) is left out; the text still saves.
    const doc = await attachmentDoc(hostId, a).catch(() => undefined);
    if (doc) attachments.push(doc);
  }
  if (!d.text.trim() && !d.quotes.length && !d.chips.length && !attachments.length) return null;
  return { v: 1, text: d.text, quotes: d.quotes, chips: d.chips, attachments };
}

async function save(key: string): Promise<void> {
  const target = sessionOfKey(key);
  if (!target) return;
  lastSave.set(key, Date.now());
  try {
    const draft = await buildDraft(key, target.hostId);
    const body = JSON.stringify(draft);
    if (sent.get(key) === body) return;
    await hostClient(target.hostId).request('draft_set', { session: target.sessionId, draft });
    sent.set(key, body);
  } catch {
    // Offline / an old PC: the local copy still has the text; the next change tries again.
  }
}

/** A change to the chat's draft: save it, throttled per chat. */
export function scheduleDraftSync(key: string): void {
  if (!sessionOfKey(key) || !loaded.has(key)) return;
  if (timers.has(key)) return; // the pending write picks up the latest state
  const wait = Math.max(0, (lastSave.get(key) ?? 0) + SAVE_THROTTLE_MS - Date.now());
  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key);
      void save(key);
    }, wait),
  );
}

/** A send: the draft is gone, on the PC too. */
export function clearHostDraft(key: string): void {
  const target = sessionOfKey(key);
  if (!target) return;
  const t = timers.get(key);
  if (t) clearTimeout(t);
  timers.delete(key);
  sent.set(key, 'null');
  void hostClient(target.hostId)
    .request('draft_set', { session: target.sessionId, draft: null })
    .catch(() => undefined);
}

function restoreAttachments(key: string, hostId: string, list: DraftAttachment[]): Promise<Map<string, string>> {
  const remap = new Map<string, string>();
  return Promise.all(
    list.map(async (a) => {
      if (a.kind === 'file') {
        const added = a.path ? addReadyFile(key, { name: a.name, mime: a.mime, size: a.size, path: a.path }) : undefined;
        if (added) remap.set(a.id, added.id);
        return;
      }
      if (!a.sha256) return;
      try {
        const got = await blobTransport().getBytes(hostId, { sha256: a.sha256 });
        const data = bytesBase64(got.bytes);
        const added = addReadyImage(key, { name: a.name, mime: a.mime || got.mime, data });
        if (added) {
          remap.set(a.id, added.id);
          uploaded.set(`${added.id}:${added.size}:${data.length}`, a.sha256);
        }
      } catch {
        // The blob is gone from the PC's store: the image chip drops out.
      }
    }),
  ).then(() => remap);
}

/** The draft's text with its chips pointing at the restored attachments (lost ones drop out). */
export function remapDraftText(text: string, attachments: readonly DraftAttachment[], remap: ReadonlyMap<string, string>): string {
  return text
    .replace(/\[image]@img:(\S+) ?/g, (all, id: string) => (remap.has(id) ? imageToken(remap.get(id)!) + (all.endsWith(' ') ? ' ' : '') : ''))
    .replace(/\[file: ([^\]\n]*)\]@att:(\S+) ?/g, (all, name: string, id: string) => {
      if (!remap.has(id)) return attachments.some((a) => a.id === id) ? '' : all;
      return attachmentToken(name, remap.get(id)!) + (all.endsWith(' ') ? ' ' : '');
    });
}

/** The chat's composer mounted: bring in the PC's draft (once per app run). */
export async function loadHostDraft(key: string): Promise<void> {
  const target = sessionOfKey(key);
  if (!target || loaded.has(key)) return;
  try {
    if (hasLiveDraft(key)) return; // this run's own draft is newer: it is saved on the next change
    const reply = await hostClient(target.hostId).request<{ draft?: HostDraft | null }>('draft_get', { session: target.sessionId });
    const draft = reply?.draft;
    if (!draft || draft.v !== 1 || typeof draft.text !== 'string' || hasLiveDraft(key)) return;
    const list = Array.isArray(draft.attachments) ? draft.attachments : [];
    const remap = await restoreAttachments(key, target.hostId, list);
    if (hasLiveDraft(key)) return; // typed into meanwhile: the user's edit wins
    markPlaced(key, [...remap.values()]);
    applyHostDraft(key, { text: remapDraftText(draft.text, list, remap), quotes: draft.quotes ?? [], chips: draft.chips ?? [] });
    sent.set(key, JSON.stringify(draft));
  } catch {
    // An old PC without drafts, or offline: local drafts only.
  } finally {
    loaded.add(key);
    // Edits made while the PC answered are saved now.
    if (hasLiveDraft(key)) scheduleDraftSync(key);
  }
}

/** Tests: forget every sync state. */
export function resetDraftSync(): void {
  for (const t of timers.values()) clearTimeout(t);
  timers.clear();
  lastSave.clear();
  uploaded.clear();
  loaded.clear();
  sent.clear();
}
