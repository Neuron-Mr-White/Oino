/**
 * Drives the "needs you" store (lib/needsYou.ts) from every paired host's
 * live chat_list (UNI-161 §3): polls on app open/resume and while the bell
 * drawer is open, turning each waiting pane into a NeedsYouItem. One shared
 * poller (startNeedsYouPoll is idempotent) so the bell works from anywhere
 * in the app, not just the Chat tab.
 */
import { chatLists, waitingTargets } from './chat/list';
import { needsYou, needsYouId, type NeedsYouItem } from './needsYou';
import type { HostInfo } from './schemas';
import { pollNtfyHistory } from './notifyPoll';
import type { NotifySettings } from '../settings/schema';

const groupForKind = (kind: string): NeedsYouItem['group'] =>
  kind === 'agent_end' || kind === 'agent_settled' ? 'finished' : 'waiting';

/** One poll pass over every paired host: loads chat_list, pushes its waiting panes in. */
export async function pollNeedsYou(hosts: readonly HostInfo[]): Promise<void> {
  const lists = chatLists();
  const store = needsYou();
  await Promise.all(
    hosts.map(async (host) => {
      await lists.load(host.id);
      const entry = lists.get(host.id);
      if (!entry?.data) return;
      const targets = waitingTargets(host.id, entry.data);
      const items: NeedsYouItem[] = targets.map((t) => ({
        id: needsYouId(host.id, t.pid, t.waiting.kind, t.waiting.since),
        hostId: host.id,
        hostName: host.name,
        pid: t.pid,
        session: t.title,
        title: t.title,
        kind: t.waiting.kind,
        group: groupForKind(t.waiting.kind),
        what: t.waiting.title,
        since: t.waiting.since,
      }));
      store.setHostItems(host.id, items);
    }),
  );
}

let timer: ReturnType<typeof setInterval> | undefined;
let listening = false;

/** UNI-161 §5: on open/resume, polls the user's own ntfy topic history
 *  (`notifyPoll.ts`) and decrypts any `unipi1:` messages into the drawer
 *  — the fallback for when no UnifiedPush distributor is installed.
 *  No-op when notifications are disabled or no server/topic is set. */
async function pollNtfyFallback(notify: NotifySettings): Promise<void> {
  if (!notify.enabled || !notify.keepListening) return;
  await pollNtfyHistory(notify.serverUrl, notify.topic, notify.token || undefined, notify.key || undefined);
}

/** Starts polling every paired host (app open/resume + every `periodMs` while
 *  visible), plus the ntfy history fallback when `getNotifySettings` is
 *  given and "Keep listening" is on. Call again any time `hosts` changes
 *  shape — it just re-reads the accessor each tick, so one call per app
 *  lifetime is enough. */
export function startNeedsYouPoll(
  getHosts: () => readonly HostInfo[],
  periodMs = 20_000,
  getNotifySettings?: () => NotifySettings,
): () => void {
  const tick = () => {
    void pollNeedsYou(getHosts());
    if (getNotifySettings) void pollNtfyFallback(getNotifySettings());
  };
  tick();
  if (!listening) {
    listening = true;
    document.addEventListener('visibilitychange', () => {
      if (!document.hidden) tick();
    });
  }
  if (timer) clearInterval(timer);
  timer = setInterval(() => {
    if (!document.hidden) tick();
  }, periodMs);
  return () => {
    if (timer) clearInterval(timer);
    timer = undefined;
  };
}
