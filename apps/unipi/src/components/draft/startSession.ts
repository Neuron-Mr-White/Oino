/**
 * Starts the draft's session (Z-19): opens a `session_new` stream in the
 * chosen folder, replays the draft's choices (model, thinking, permission,
 * plan — in that order, the same commands the chat's own controls send) and
 * sends the first prompt over that very stream.
 *
 * Why over the new stream: a new session has no file until its first
 * message, and the host resolves `session_open` by its file — the chat the
 * app routes to can only open it once the prompt is in. The prompt is sent
 * here, then the chat opens it (and shows the prompt from the host's own
 * entries); this stream lingers a few seconds so the runtime is never left
 * without a subscriber in between.
 */
import { toAppError, type AppError } from '@unipi/protocol';
import { hostClient } from '../../lib/host/factory';
import type { HostClient, RpcCommand, SessionSnapshot } from '../../lib/host/client';
import type { OutboxImage } from '../../lib/session/types';
import type { DraftChoices } from './draftSession';

export type StartRequest = {
  cwd?: string;
  text: string;
  images: OutboxImage[];
  choices: DraftChoices;
};

/** How long the creating stream stays open after the prompt went in. */
export const LINGER_MS = 6000;
const SNAPSHOT_TIMEOUT_MS = 20_000;

export async function startSession(hostId: string, req: StartRequest, client: HostClient = hostClient(hostId)): Promise<{ ok: true; sessionId: string } | { ok: false; error: AppError }> {
  let settle: ((snap: SessionSnapshot) => void) | undefined;
  let fail: ((error: AppError) => void) | undefined;
  const ready = new Promise<SessionSnapshot>((resolve, reject) => {
    settle = resolve;
    fail = (error) => reject(error);
  });
  const stream = client.openSession(
    { kind: 'new', ...(req.cwd ? { cwd: req.cwd } : {}) },
    {
      snapshot: (snap) => settle?.(snap),
      events: () => {},
      closed: (error) => fail?.(error ?? toAppError(new Error('The host closed the new session'))),
    },
  );
  const timer = setTimeout(() => fail?.(toAppError(new Error('The host did not start the session in time'))), SNAPSHOT_TIMEOUT_MS);
  try {
    const snap = await ready;
    clearTimeout(timer);
    const cmd = (c: RpcCommand) => stream.cmd(c);
    const { model, thinking, permission, plan } = req.choices;
    // A failed preference never blocks the prompt: the chat shows what runs.
    if (model) await cmd({ type: 'set_model', provider: model.provider, modelId: model.modelId }).catch(() => undefined);
    if (thinking) await cmd({ type: 'set_thinking_level', level: thinking }).catch(() => undefined);
    if (permission) await cmd({ type: 'prompt', message: `/permission ${permission}` }).catch(() => undefined);
    if (plan) await cmd({ type: 'prompt', message: '/plan on' }).catch(() => undefined);
    const images = req.images.map((img) => ({ type: 'image', data: img.data, mimeType: img.mimeType }));
    await cmd({ type: 'prompt', message: req.text, ...(images.length ? { images } : {}) });
    setTimeout(() => stream.close(), LINGER_MS);
    return { ok: true, sessionId: snap.session_id };
  } catch (error) {
    clearTimeout(timer);
    stream.close();
    return { ok: false, error: toAppError(error) };
  }
}
