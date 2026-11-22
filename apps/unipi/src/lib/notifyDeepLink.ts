/**
 * G-12 (WP-A19): routes a `unipi://s/<host>/<session>` deep link — the form
 * every push (needs-you, cron run, finished) and the mascot overlay tap
 * opens — to that session's route. The v1 `unipi://chat?host=&session=`
 * query form (and its `pid=`/`dialog=` keys) is still parsed for old
 * notifications, and `readPendingAction()` reads the one action queued by
 * the native side (`NotificationBridge.takePendingAction`) — an ask_user
 * answer or permission allow/deny tapped while the app was closed.
 */
import { z } from 'zod';

export const ChatDeepLinkSchema = z.object({
  host: z.string().optional(),
  /** The oino session id (v2). */
  session: z.string().min(1).optional(),
  pid: z.coerce.number().int().optional(),
  dialog: z.coerce.number().int().optional(),
});
export type ChatDeepLink = z.infer<typeof ChatDeepLinkSchema>;

/** Parses `unipi://s/<host>/<session>` (G-12) or `unipi://chat?host=&pid=&dialog=`; `undefined` for any other URL. */
export function parseChatDeepLink(raw: string): ChatDeepLink | undefined {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return undefined;
  }
  if (url.protocol !== 'unipi:') return undefined;
  // v2 path form: unipi://s/<host>/<session> (both URL-encoded).
  if (url.hostname === 's') {
    const [host, session, ...rest] = url.pathname.split('/').filter(Boolean).map(decodeURIComponent);
    if (!host || !session || rest.length > 0) return undefined;
    return { host, session };
  }
  if (url.hostname !== 'chat') return undefined;
  const result = ChatDeepLinkSchema.safeParse({
    host: url.searchParams.get('host') ?? undefined,
    session: url.searchParams.get('session') ?? undefined,
    pid: url.searchParams.get('pid') ?? undefined,
    dialog: url.searchParams.get('dialog') ?? undefined,
  });
  return result.success ? result.data : {};
}

export const PendingActionSchema = z.object({
  notificationId: z.number().int(),
  action: z.string(),
});
export type PendingAction = z.infer<typeof PendingActionSchema>;

/** Reads window.UniPiNotify.takePendingAction() if the native bridge is
 *  present (Android only); `undefined` everywhere else or on bad JSON. */
export function readPendingAction(): PendingAction | undefined {
  const bridge = (window as unknown as { UniPiNotify?: { takePendingAction?: () => string | null } }).UniPiNotify;
  if (!bridge?.takePendingAction) return undefined;
  const raw = bridge.takePendingAction();
  if (!raw) return undefined;
  try {
    const parsed = PendingActionSchema.safeParse(JSON.parse(raw));
    return parsed.success ? parsed.data : undefined;
  } catch {
    return undefined;
  }
}
