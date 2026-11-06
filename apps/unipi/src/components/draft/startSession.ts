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

export async function startSession(hostId: string, req: StartRequest, client: HostClient = hostClient(hostId)): Promise<{ ok: true; sessionId: string } | { ok: false; error: AppError }> {
  let settle: ((snap: SessionSnapshot) => void) | undefined;
  /** Resolves when the prompt's user message is in the session (its file exists from then on). */
  let persisted: (() => void) | undefined;
  const userIn = new Promise<void>((resolve) => (persisted = resolve));
  /** Set right before the first prompt: a replayed `/permission` never counts. */
  let armed = false;
  let fail: ((error: AppError) => void) | undefined;
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
          const ev = (item as { event?: { type?: string; message?: { role?: string } } }).event;
          if (armed && ev?.type === 'message_end' && ev.message?.role === 'user') persisted?.();
        }
      },
      closed: (error) => fail?.(error ?? toAppError(new Error('The host closed the new session'))),
    },
  );
  const timer = setTimeout(() => fail?.(toAppError(new Error('The host did not start the session in time'))), SNAPSHOT_TIMEOUT_MS);
  try {
    const snap = await ready;
    clearTimeout(timer);
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
    const images = req.images.map((img) => ({ type: 'image', data: img.data, mimeType: img.mimeType }));
    await cmd({ type: 'prompt', message: req.text, ...(images.length ? { images } : {}) });
    // The chat opens the session by its file, written with the user message:
    // route only once that message is in (bounded — a slow host still routes).
    await Promise.race([userIn, new Promise((r) => setTimeout(r, PERSIST_WAIT_MS))]);
    setTimeout(() => stream.close(), LINGER_MS);
    return { ok: true, sessionId: snap.session_id };
  } catch (error) {
    clearTimeout(timer);
    stream.close();
    return { ok: false, error: toAppError(error) };
  }
}
