/**
 * Starts the draft's session (Z-19): opens a `session_new` stream in the
 * chosen folder, replays the draft's choices (model, thinking, permission,
 * plan — in that order, the same commands the chat's own controls send) and
 * sends the first prompt over that very stream.
 *
 * Why over the new stream: a new session has no file until its first
 * message, and the host resolves `session_open` by its file — the chat the
 * app routes to can only open it once the prompt is in (the prompt's ack
 * comes before its user message is appended: routing on the ack alone got an
 * `E_INTERNAL` open). So the prompt is sent here, the route waits for its
 * user `message_end`, then the chat opens the session and shows the prompt
 * from the host's own entries; this stream lingers a few seconds so the
 * runtime is never left without a subscriber in between.
 */
import { toAppError, type AppError } from '@unipi/protocol';
import { hostClient } from '../../lib/host/factory';
import type { HostClient, RpcCommand, SessionSnapshot } from '../../lib/host/client';
import type { OutboxImage } from '../../lib/session/types';
import type { DraftChoices } from './draftSession';
import { routeNotify } from '../../lib/reports';
import { FUSION_SET_PAIR } from '../../lib/model/fusion';

export type StartRequest = {
  cwd?: string;
  text: string;
  images: OutboxImage[];
  choices: DraftChoices;
};

/** How long the creating stream stays open after the prompt went in. */
export const LINGER_MS = 6000;
const SNAPSHOT_TIMEOUT_MS = 20_000;
/** Longest wait for the first user message to land before routing anyway. */
const PERSIST_WAIT_MS = 8000;

/** Same cut-off as a chat's send (`lib/session/store.ts`): over this much base64 the images go as blob refs. */
const INLINE_IMAGES_MAX = 700 * 1024;

/**
 * The prompt's images. A pasted screenshot is routinely past the host's frame limit when inlined,
 * so — exactly as a chat does — big sets are uploaded first (four at a time, order kept) and sent by reference.
 */
async function promptImages(hostId: string, images: readonly OutboxImage[]): Promise<Array<Record<string, string>>> {
  if (images.reduce((n, img) => n + img.data.length, 0) <= INLINE_IMAGES_MAX) {
    return images.map((img) => ({ type: 'image', data: img.data, mimeType: img.mimeType }));
  }
  const { blobTransport } = await import('../../lib/blob');
  const out: Array<Record<string, string>> = [];
  for (let i = 0; i < images.length; i += 4) {
    out.push(
      ...(await Promise.all(
        images.slice(i, i + 4).map(async (img) => {
          const put = await blobTransport().putBytes(hostId, Uint8Array.from(atob(img.data), (c) => c.charCodeAt(0)), img.mimeType);
          return { type: 'image', mediaRef: put.sha256, mimeType: img.mimeType };
        }),
      )),
    );
  }
  return out;
}

export async function startSession(hostId: string, req: StartRequest, client: HostClient = hostClient(hostId)): Promise<{ ok: true; sessionId: string } | { ok: false; error: AppError }> {
  let settle: ((snap: SessionSnapshot) => void) | undefined;
  /** Resolves when the prompt's user message is in the session (its file exists from then on). */
  let persisted: (() => void) | undefined;
  const userIn = new Promise<void>((resolve) => (persisted = resolve));
  /** Set right before the first prompt: a replayed `/permission` never counts. */
  let armed = false;
  let handedOff = false;
  let fail: ((error: AppError) => void) | undefined;
  /** The new session's id once the host answered, and whether the first prompt was refused (D3). */
  let created: string | undefined;
  let refused = false;
  /**
   * N8: the command's own notice ("Swarm not started: …") reaches this stream before the prompt's reply does. A refusal
   * shows that same reason inline in the draft, so notices that arrive while the first prompt is out wait here and are
   * dropped if they just repeat the refusal; everything else is routed as before.
   */
  let holding = false;
  let refusedReason: string | undefined;
  const held: Array<{ message: string; kind: 'error' | 'warning' | 'info'; key: string }> = [];
  const flushHeld = (except?: string) => {
    holding = false;
    for (const n of held.splice(0)) if (n.message.trim() !== except?.trim()) routeNotify(n.message, n.kind, n.key);
  };
  const ready = new Promise<SessionSnapshot>((resolve, reject) => {
    settle = resolve;
    fail = (error) => reject(error);
  });
  const stream = client.openSession(
    { kind: 'new', ...(req.cwd ? { cwd: req.cwd } : {}) },
    {
      snapshot: (snap) => settle?.(snap),
      events: (items) => {
        for (const item of items) {
          const ev = item.event as Record<string, unknown>;
          // The creating stream receives command notices before Conversation mounts.
          // It used to discard them; effects are one-shot and absent on the next open.
          if (!handedOff && ev?.type === 'extension_ui_request' && ev.method === 'notify' && typeof ev.message === 'string') {
            const kind = ev.notifyType === 'error' ? 'error' : ev.notifyType === 'warning' || ev.notifyType === 'warn' ? 'warning' : 'info';
            const key = `${hostId}/${item.session}`;
            if (holding) held.push({ message: ev.message, kind, key });
            else if (ev.message.trim() !== refusedReason?.trim()) routeNotify(ev.message, kind, key);
          }
          if (armed && ev?.type === 'message_end' && (ev.message as { role?: string } | undefined)?.role === 'user') persisted?.();
        }
      },
      closed: (error) => fail?.(error ?? toAppError(new Error('The host closed the new session'))),
    },
  );
  const timer = setTimeout(() => fail?.(toAppError(new Error('The host did not start the session in time'))), SNAPSHOT_TIMEOUT_MS);
  try {
    const snap = await ready;
    clearTimeout(timer);
    created = snap.session_id;
    const cmd = (c: RpcCommand) => stream.cmd(c);
    const { model, thinking, permission, plan, fusion, autonomy } = req.choices;
    // A failed preference never blocks the prompt: the chat shows what runs.
    // D-02: only what differs from the new session's own start — a replay of the same
    // model / level would put "Model → … / Thinking → …" rows atop a fresh chat.
    const started = (snap.state ?? {}) as { model?: { provider?: string; id?: string }; thinkingLevel?: string };
    const sameModel = !!model && started.model?.id === model.modelId && (!started.model.provider || started.model.provider === model.provider);
    if (fusion) {
      // The draft's Fusion pair, through `unipi.fusion`'s own service (the chat picker's
      // path): it switches pi to the lead and sets both efforts, so no set_model /
      // thinking replay goes first.
      await cmd({ type: '_oino/service_call', name: FUSION_SET_PAIR, payload: { active: { kind: 'fusion', ...fusion } } }).catch(() => undefined);
    } else {
      if (model && !sameModel) await cmd({ type: 'set_model', provider: model.provider, modelId: model.modelId }).catch(() => undefined);
      if (thinking && thinking !== started.thinkingLevel) await cmd({ type: 'set_thinking_level', level: thinking }).catch(() => undefined);
    }
    if (autonomy) await cmd({ type: 'set_autonomy', level: autonomy });
    if (permission) await cmd({ type: 'prompt', message: `/permission ${permission}` }).catch(() => undefined);
    if (plan) await cmd({ type: 'prompt', message: '/plan on' }).catch(() => undefined);
    armed = true;
    const images = await promptImages(hostId, req.images);
    holding = true;
    const reply = await cmd({ type: 'prompt', message: req.text, ...(images.length ? { images } : {}) }) as { disposition?: string; reason?: string } | undefined;
    if (reply?.disposition === 'refused') {
      refused = true;
      refusedReason = reply.reason;
      flushHeld(reply.reason);
      throw new Error(reply.reason ?? 'Command refused');
    }
    flushHeld();
    // The chat opens the session by its file, written with the user message:
    // route only once that message is in (bounded — a slow host still routes).
    await Promise.race([userIn, new Promise((r) => setTimeout(r, PERSIST_WAIT_MS))]);
    handedOff = true;
    setTimeout(() => stream.close(), LINGER_MS);
    return { ok: true, sessionId: snap.session_id };
  } catch (error) {
    clearTimeout(timer);
    flushHeld();
    stream.close();
    // D3: a refused first command leaves the empty chat the stream created (a command echo, no
    // message). The draft gets its text back, so the chat must not stay in the lists — and a
    // resend would otherwise make a second one. Best effort: a failure here only leaves it listed.
    if (refused && created) {
      const session = created;
      await client.request('session_archive', { session }).catch(() => undefined);
      await client.request('session_delete', { session }).catch(() => undefined);
    }
    return { ok: false, error: toAppError(error) };
  }
}
