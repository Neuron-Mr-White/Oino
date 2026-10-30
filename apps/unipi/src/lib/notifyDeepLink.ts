/**
 * UNI-161 §2/§3: routes a `unipi://chat?host=&pid=&dialog=` deep link
 * (tapped from a notification, native or a UnifiedPush background
 * delivery) to the right chat, and reads one pending action queued by
 * the native side (`NotificationBridge.takePendingAction`) — an ask_user
 * answer or permission allow/deny tapped while the app was closed.
 */
import { z } from 'zod';

export const ChatDeepLinkSchema = z.object({
  host: z.string().optional(),
  pid: z.coerce.number().int().optional(),
  dialog: z.coerce.number().int().optional(),
});
export type ChatDeepLink = z.infer<typeof ChatDeepLinkSchema>;

/** Parses `unipi://chat?host=&pid=&dialog=`; `undefined` for any other URL. */
export function parseChatDeepLink(raw: string): ChatDeepLink | undefined {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return undefined;
  }
  if (url.protocol !== 'unipi:' || url.hostname !== 'chat') return undefined;
  const result = ChatDeepLinkSchema.safeParse({
    host: url.searchParams.get('host') ?? undefined,
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
