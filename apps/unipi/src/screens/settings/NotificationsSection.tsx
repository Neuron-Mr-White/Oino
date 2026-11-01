/**
 * Notifications section (WP-A24): desktop notifications (this app), phone
 * notifications (UNI-161: the user's own ntfy server/topic/token + the
 * per-pairing E2E key + background delivery through UnifiedPush) and — P2-03,
 * the one notifications place — the PC's `notify` extension settings (event
 * matrix, Gotify / Telegram / ntfy channels, silence & renotify), rendered by
 * the same form engine as Settings › Extensions (whose `notify` row now opens
 * here instead of a second copy).
 *
 * The phone switch is about *receiving on this phone*. Sending is the PC's
 * side: once a phone is paired, the PC pushes "needs you" and "agent finished"
 * to it by default; which events notify is the event matrix below.
 */
import { For, Show, createMemo, createResource, createSignal, onCleanup, onMount } from 'solid-js';
import { Bell, ChevronRight } from 'lucide-solid';
import { NamespaceForm } from '../../ui/settings/form';
import { settingsSchema } from '../../ui/settings/data';
import type { Namespace } from '../../ui/settings/types';
import { LOCAL_HOST } from '../../lib/host/client';
import { z } from 'zod';
import { call } from '../../lib/ipc';
import { can } from '../../lib/platform';
import { desktopNotifyOn, setDesktopNotifyOn } from '../../lib/desktop';
import { pushStatus, registerPush, syncPushEndpoint, type PushStatus } from '../../lib/pushRegister';
import { useSettings } from '../../settings/context';
import { useHosts } from '../../hosts/context';
import { Row, Section } from './rows';
import { SwitchField } from '../../components/Switch';

const NotifyConfigSchema = z.object({
  server: z.string().optional(),
  topic: z.string().optional(),
  token: z.string().optional(),
  key: z.string(),
  app_detail: z.enum(['minimal', 'full']).default('minimal'),
});

export function NotificationsSection() {
  const [settings, update] = useSettings();
  const hosts = useHosts();
  const [desktopOn, setDesktopOn] = createSignal(desktopNotifyOn());
  const [testState, setTestState] = createSignal<'idle' | 'sending' | 'ok' | 'error'>('idle');
  const [testMessage, setTestMessage] = createSignal('');

  function currentHostId(): string | undefined {
    return hosts.selectedId() ?? hosts.hosts()[0]?.id;
  }

  async function sendTest() {
    setTestState('sending');
    const hostId = currentHostId();
    if (hostId) {
      const result = await call('notify_test', { hostId }, z.unknown());
      if (result.ok) {
        setTestState('ok');
        setTestMessage('Sent — check your phone.');
      } else {
        setTestState('error');
        setTestMessage(result.error.message);
      }
      return;
    }
    const { serverUrl, topic, token } = settings.notify;
    if (!serverUrl || !topic) {
      setTestState('error');
      setTestMessage('Set a server URL and topic first.');
      return;
    }
    try {
      const headers: Record<string, string> = { 'Content-Type': 'application/json' };
      if (token) headers.Authorization = `Bearer ${token}`;
      const res = await fetch(serverUrl.replace(/\/$/, ''), {
        method: 'POST',
        headers,
        body: JSON.stringify({ topic, title: 'UniPi', message: 'Test notification from the app settings.', priority: 3 }),
      });
      if (!res.ok) throw new Error(`ntfy error ${res.status}`);
      setTestState('ok');
      setTestMessage('Sent — check your phone.');
    } catch (error) {
      setTestState('error');
      setTestMessage(String((error as Error)?.message ?? error));
    }
  }

  /** "Get from this PC" (UNI-161): asks the connected host for its own ntfy
   *  server/topic/token plus the per-pairing E2E key. */
  async function getFromPc() {
    const hostId = currentHostId();
    if (!hostId) {
      setTestState('error');
      setTestMessage('Pair a host first (Settings → Hosts).');
      return;
    }
    setTestState('sending');
    const result = await call('notify_config', { hostId }, NotifyConfigSchema);
    if (!result.ok) {
      setTestState('error');
      setTestMessage(result.error.message);
      return;
    }
    const { server, topic, token, key } = result.value;
    update('notify', {
      ...settings.notify,
      enabled: true,
      serverUrl: server ?? settings.notify.serverUrl,
      topic: topic ?? settings.notify.topic,
      token: token ?? settings.notify.token,
      key,
      keySourceHostId: hostId,
    });
    setTestState('ok');
    setTestMessage(server ? 'Configured from the PC.' : 'Key configured — set a server/topic on the PC to finish.');
  }

  // P2-03: the PC's notify extension settings live here (one place). The phone reads its paired PC's.
  const notifyHost = () => (can('localSharing') ? LOCAL_HOST : (currentHostId() ?? LOCAL_HOST));
  const [namespaces] = createResource(notifyHost, async (hostId): Promise<Namespace[]> => {
    const result = await settingsSchema(hostId);
    return result.ok ? result.value : [];
  });
  const notifyNs = createMemo(() => (namespaces.latest ?? []).find((ns) => ns.namespace === 'notify'));
  const toEvents = () => scrollToNotifySection('events');

  return (
    <>
    <Section title="Notifications" id="sec-notifications" icon={<Bell size={14} />}>
      <Show when={can('nativeNotifications')}>
        <Row label="Desktop notifications" description="A system notification when the agent needs you or finishes, while UniPi isn't in front">
          <SwitchField label="Desktop notifications" checked={desktopOn()} onChange={(v) => (setDesktopNotifyOn(v), setDesktopOn(v))} />
        </Row>
      </Show>
      <Show when={can('notifications')}>
        <Row
          label="Receive on this phone"
          description="Show the alerts your paired PCs push through your ntfy server. Each PC sends “needs you” and “agent finished” by default once this phone is paired; turning this off only silences this phone."
        >
          <SwitchField label="Receive on this phone" checked={settings.notify.enabled} onChange={(v) => update('notify.enabled', v)} />
        </Row>
        <Show when={notifyNs()}>
          <button type="button" class="settings-namespace-row" data-testid="notify-events-link" onClick={toEvents}>
            <span class="min-w-0">
              <span class="block text-[15px] text-text">Which events notify</span>
              <span class="block text-xs text-text-dim">The PC’s event matrix: needs you, agent finished, errors…</span>
            </span>
            <ChevronRight size={15} class="text-text-dim shrink-0" />
          </button>
        </Show>
        <Show when={settings.notify.enabled}>
          <div class="px-4 py-3 space-y-3">
            <div>
              <label class="text-xs text-text-dim block mb-1" for="ntfy-server">Server URL</label>
              <input
                id="ntfy-server"
                type="url"
                class="w-full rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text"
                value={settings.notify.serverUrl}
                onInput={(e) => update('notify.serverUrl', e.currentTarget.value)}
                placeholder="https://ntfy.sh"
              />
            </div>
            <div>
              <label class="text-xs text-text-dim block mb-1" for="ntfy-topic">Topic</label>
              <input
                id="ntfy-topic"
                type="text"
                class="w-full rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text"
                value={settings.notify.topic}
                onInput={(e) => update('notify.topic', e.currentTarget.value)}
                placeholder="your-private-topic"
              />
            </div>
            <div>
              <label class="text-xs text-text-dim block mb-1" for="ntfy-token">Access token (optional)</label>
              <input
                id="ntfy-token"
                type="password"
                class="w-full rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text"
                value={settings.notify.token}
                onInput={(e) => update('notify.token', e.currentTarget.value)}
                placeholder="tk_..."
              />
            </div>
            <div class="flex gap-2">
              <button type="button" class="rounded-pill bg-shell text-face px-4 py-2 text-sm font-medium" onClick={() => void sendTest()} disabled={testState() === 'sending'}>
                {testState() === 'sending' ? 'Sending…' : 'Test'}
              </button>
              <button type="button" class="rounded-pill bg-surface-2 text-text-muted px-4 py-2 text-sm" onClick={() => void getFromPc()}>
                Get from this PC
              </button>
            </div>
            <Show when={testMessage()}>
              <p class={`text-xs ${testState() === 'error' ? 'text-shell' : 'text-text-dim'}`} role="status">{testMessage()}</p>
            </Show>
            <Show when={settings.notify.key}>
              <p class="text-xs text-text-dim" role="status">End-to-end encrypted — the server only sees ciphertext.</p>
            </Show>
          </div>
        </Show>
        <PushStatusRow />
        <Row label="Keep listening" description="Poll the topic on resume + while the app is open (fallback when no distributor is installed)">
          <SwitchField label="Keep listening" checked={settings.notify.keepListening} onChange={(v) => update('notify.keepListening', v)} />
        </Row>
      </Show>
    </Section>
    <Show when={notifyNs()}>
      {(ns) => (
        <Section title={can('localSharing') ? 'What this PC sends' : 'What your PC sends'} navTitle="Notifications" id="sec-notify-pc" icon={<Bell size={14} />}>
          <p class="settings-page-blurb">Events, channels (ntfy, Gotify, Telegram, desktop) and quiet hours of the PC’s notify extension.</p>
          <NamespaceForm namespace={ns()} hostId={notifyHost()} bare flat testId="notify-pc-form" />
        </Section>
      )}
    </Show>
    </>
  );
}

/** Scrolls to one section of the embedded notify form (`events`, `platforms`, …) and flashes it. */
export function scrollToNotifySection(id: string): void {
  const el = document.querySelector<HTMLElement>(`[data-testid="notify-pc-form"] [data-section-id="${id}"]`) ?? document.getElementById('sec-notify-pc');
  if (!el) return;
  el.scrollIntoView?.({ block: 'start', behavior: 'smooth' });
  el.classList.remove('settings-flash-target');
  void el.offsetWidth;
  el.classList.add('settings-flash-target');
  window.setTimeout(() => el.classList.remove('settings-flash-target'), 1400);
}

/** UNI-161: background delivery through the ntfy app (UnifiedPush). */
function PushStatusRow() {
  const hosts = useHosts();
  const [status, setStatus] = createSignal<PushStatus>(pushStatus());
  const [busy, setBusy] = createSignal(false);
  const refresh = () => setStatus(pushStatus());
  onMount(() => {
    const t = setInterval(refresh, 2000);
    onCleanup(() => clearInterval(t));
  });
  const register = async () => {
    setBusy(true);
    registerPush();
    // The endpoint arrives asynchronously from the distributor.
    for (let i = 0; i < 20 && !pushStatus().endpoint; i++) await new Promise((r) => setTimeout(r, 500));
    await syncPushEndpoint(hosts.hosts().map((h) => h.id));
    refresh();
    setBusy(false);
  };
  const text = () => {
    const st = status();
    if (st.state === 'unsupported') return 'Only on the Android app';
    if (st.state === 'no_distributor') return 'Install the ntfy app to get notifications while UniPi is closed';
    if (st.state === 'registered' && st.endpoint) return `On, through ${st.distributor?.split('.').pop() ?? 'the ntfy app'}`;
    return 'Not set up yet';
  };
  return (
    <Row label="Background delivery" description={text()}>
      <Show when={status().state !== 'unsupported' && status().state !== 'no_distributor'}>
        <button type="button" class="rounded-pill bg-surface-2 text-text px-4 py-2 text-sm" disabled={busy()} onClick={() => void register()}>
          {busy() ? 'Setting up…' : status().state === 'registered' ? 'Re-register' : 'Turn on'}
        </button>
      </Show>
    </Row>
  );
}
