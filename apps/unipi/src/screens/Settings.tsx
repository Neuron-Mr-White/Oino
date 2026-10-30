import { Layers, PictureInPicture2 } from 'lucide-solid';
import { Type, AlignJustify, TextCursor, Sparkle, ScrollText, Smartphone, Sun, History, SquareTerminal, Vibrate, SpellCheck, PanelBottom, RadioTower, Route, KanbanSquare, Archive, Bell, Link2 } from 'lucide-solid';
import { HostsSettings } from '../components/HostsSettings';
import { SwitchField } from '../components/Switch';
import { SharingSettings } from '../components/SharingSettings';
import { PushAccess } from '../components/PushAccess';
import { ShortcutsSettings } from '../components/command/ShortcutsSettings';
import { LanguageServers } from '../components/LanguageServers';
import { For, Show, createContext, createResource, createSignal, onCleanup, onMount, useContext, type JSX } from 'solid-js';
import { desktopShell } from '../lib/layout';
import { can } from '../lib/platform';
import { desktopNotifyOn, setDesktopNotifyOn } from '../lib/desktop';
import {
  Cpu,
  FolderKanban,
  Info,
  Keyboard,
  Minus,
  MonitorSmartphone,
  Plus,
  Server,
  ChevronDown,
  ChevronUp,
} from 'lucide-solid';
import { z } from 'zod';
import { call } from '../lib/ipc';
import { UpdateCard } from '../components/UpdateCard';
import { DesktopUpdateCard, IosUpdateCard } from '../components/DesktopUpdate';
import { Requires } from '../components/Requires';
import { platform } from '../lib/platform';
import { useSettings, useSettingsApi } from '../settings/context';
import { TOOLBAR_KEYS, type ToolbarKey } from '../settings/schema';
import { useHosts } from '../hosts/context';
import { Unicrab } from '../components/Unicrab';
import { floatingMascotOn, setFloatingMascotOn } from '../components/FloatingBell';
import { MASCOT_WINDOW_SIZES, mascotWindowOn, mascotWindowSize, setMascotWindowSize, type MascotWindowSize } from '../lib/mascot';
import { overlayRunning, setOverlay, syncMascotWindow } from '../lib/mascotNative';
import { overlayBridge } from '../lib/mascotFeed';
import { pushStatus, registerPush, syncPushEndpoint, type PushStatus } from '../lib/pushRegister';

/** UNI-117 desktop: Settings is two columns — the section list | the picked section. */
const SectionNav = createContext<{ active: () => string | undefined }>();

/** Section wrapper: a titled card in the settings list (DESIGN.md §3.10). */
function Section(props: { title: string; icon: JSX.Element; children: JSX.Element }) {
  const nav = useContext(SectionNav);
  const hide = () => desktopShell() && !!nav?.active() && nav.active() !== props.title;
  return (
    <section class="mb-7" data-section={props.title} hidden={hide() ? true : undefined}>
      <h2 class="section-label settings-section-label">
        {props.icon}
        {props.title}
      </h2>
      <div class="settings-group rounded-card bg-surface-1">{props.children}</div>
    </section>
  );
}

const rowIcons: Record<string, typeof Info> = {
 'Font size':Type,'Line height':AlignJustify,'Cursor style':TextCursor,'Cursor blink':Sparkle,
 'Scrollback':ScrollText,'Phone-size policy':Smartphone,'Keep screen on':Sun,'Reopen last session':History,
 'Haptics':Vibrate,'Composer autocorrect':SpellCheck,'Toolbar':PanelBottom,
 'Relay':RadioTower,'Path preference':Route,'Default project':KanbanSquare,'Show backlog':Archive,
 'Version':Info,'Platform':Smartphone,'Protocol':SquareTerminal,'Licences':ScrollText,
 'Enable phone notifications':Bell,'Server URL':Link2,'Topic':Bell,'Access token':Link2,
 'Keep listening':RadioTower,'Status':Bell,'Desktop notifications':Bell,
 'Floating crab':Smartphone,'Over other apps':Layers,'Mascot window':PictureInPicture2,
};
function Row(props: { label: string; description?: string; children: JSX.Element }) {
  const Icon = rowIcons[props.label] || Info;
  return (
    <div class="settings-row">
      <span class="settings-icon"><Icon size={18}/></span>
      <div class="min-w-0">
        <p class="text-[15px] text-text">{props.label}</p>
        <Show when={props.description}>
          <p class="text-xs text-text-dim mt-0.5">{props.description}</p>
        </Show>
      </div>
      <div class="shrink-0 text-right text-text-muted">{props.children}</div>
    </div>
  );
}

function Stepper(props: {
  value: number;
  min: number;
  max: number;
  step: number;
  label: string;
  format?: (v: number) => string;
  onChange: (v: number) => void;
}) {
  const fmt = props.format ?? ((v: number) => String(v));
  const dec = () => props.onChange(Math.max(props.min, round(props.value - props.step)));
  const inc = () => props.onChange(Math.min(props.max, round(props.value + props.step)));
  function round(v: number) {
    // Avoid float drift (e.g. 1.2 - 0.1 = 1.0999999...).
    return Math.round(v * 100) / 100;
  }
  return (
    <div class="settings-stepper" role="group" aria-label={props.label}>
      <button type="button" class="rounded-input bg-surface-2 w-8 h-8 flex items-center justify-center" onClick={dec} aria-label={`Decrease ${props.label}`}>
        <Minus size={14} />
      </button>
      <span class="w-12 text-center text-sm tabular-nums">{fmt(props.value)}</span>
      <button type="button" class="rounded-input bg-surface-2 w-8 h-8 flex items-center justify-center" onClick={inc} aria-label={`Increase ${props.label}`}>
        <Plus size={14} />
      </button>
    </div>
  );
}

function Segmented<T extends string>(props: { value: T; options: readonly T[]; labels?: Record<T, string>; onChange: (v: T) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={props.label} class="flex rounded-pill bg-surface-3 p-1 gap-1">
      <For each={props.options}>
        {(opt) => (
          <button
            type="button"
            role="radio"
            aria-checked={props.value === opt}
            class={`px-3 py-1 text-xs rounded-pill transition-colors ${props.value === opt ? 'bg-surface-2 text-text' : 'text-text-muted'}`}
            onClick={() => props.onChange(opt)}
          >
            {props.labels?.[opt] ?? opt}
          </button>
        )}
      </For>
    </div>
  );
}

const TOOLBAR_LABELS: Record<ToolbarKey, string> = {
  ctrl: 'Ctrl',
  shift: 'Shift',
  opt: 'Opt',
  history: 'History',
  esc: 'Esc',
  tab: 'Tab',
  arrows: 'Arrows',
  ctrl_c: 'Ctrl+C',
  paste: 'Paste',
  shortcuts: 'Shortcuts',
  composer: 'Composer',
  keyboard: 'Keyboard',
  enter: '⏎',
  backspace: '⌫',
  home: 'Home',
  end: 'End',
  pgup: 'PgUp',
  pgdn: 'PgDn',
};

/** Editable toolbar order. `ctrl` is pinned first and cannot move or be removed. */
function ToolbarEditor() {
  const [settings, update] = useSettings();
  const toolbar = () => settings.input.toolbar;

  function move(index: number, dir: -1 | 1) {
    if (index === 0) return; // ctrl pinned
    const target = index + dir;
    if (target <= 0 || target >= toolbar().length) return;
    const next = [...toolbar()];
    [next[index], next[target]] = [next[target], next[index]];
    update('input.toolbar', next);
  }

  function toggle(key: ToolbarKey) {
    if (key === 'ctrl') return; // always present
    const current = toolbar();
    const next = current.includes(key) ? current.filter((k) => k !== key) : [...current, key];
    update('input.toolbar', next);
  }

  return (
    <div class="px-4 py-3">
      <p class="text-sm text-text mb-3">Toolbar</p>
      <ul class="space-y-2 mb-4">
        <For each={toolbar()}>
          {(key, index) => (
            <li class="flex items-center gap-2 bg-surface-2 rounded-input px-3 py-2">
              <span class="text-sm flex-1">{TOOLBAR_LABELS[key]}</span>
              <Show when={key !== 'ctrl'}>
                <button type="button" aria-label={`Move ${TOOLBAR_LABELS[key]} up`} class="text-text-muted" disabled={index() <= 1} onClick={() => move(index(), -1)}>
                  <ChevronUp size={16} />
                </button>
                <button type="button" aria-label={`Move ${TOOLBAR_LABELS[key]} down`} class="text-text-muted" disabled={index() >= toolbar().length - 1} onClick={() => move(index(), 1)}>
                  <ChevronDown size={16} />
                </button>
                <button type="button" aria-label={`Remove ${TOOLBAR_LABELS[key]}`} class="text-text-dim" onClick={() => toggle(key)}>
                  <Minus size={16} />
                </button>
              </Show>
            </li>
          )}
        </For>
      </ul>
      <p class="text-xs text-text-dim mb-2">Available keys</p>
      <div class="flex flex-wrap gap-2">
        <For each={TOOLBAR_KEYS.filter((k) => k !== 'ctrl' && !toolbar().includes(k))}>
          {(key) => (
            <button type="button" class="rounded-pill bg-surface-2 px-3 py-1 text-xs text-text-muted flex items-center gap-1" onClick={() => toggle(key)}>
              <Plus size={12} />
              {TOOLBAR_LABELS[key]}
            </button>
          )}
        </For>
      </div>
    </div>
  );
}

/** Phone notifications (UNI-161): the user's own ntfy server/topic/token,
 *  plus the per-pairing E2E key once "Get from this PC" (`notify_config`)
 *  has run. "Test" (`notify_test`) proves the whole encrypt -> ntfy -> app
 *  path through a paired host; falls back to a direct POST to the app's
 *  own configured server when no host is paired yet (pre-pairing path). */
const NotifyConfigSchema = z.object({
  server: z.string().optional(),
  topic: z.string().optional(),
  token: z.string().optional(),
  key: z.string(),
  app_detail: z.enum(['minimal', 'full']).default('minimal'),
});

function NotifySettings() {
  const [settings, update] = useSettings();
  const hosts = useHosts();
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
      setTestMessage('Pair a host first (Settings -> Hosts).');
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

  return (
    <Section title="Phone notifications" icon={<Bell size={14} />}>
      <Row label="Enable phone notifications" description="Push alerts via your own ntfy server when pi needs you">
        <SwitchField label="Enable phone notifications" checked={settings.notify.enabled} onChange={(v) => update('notify.enabled', v)} />
      </Row>
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
    </Section>
  );
}

const MODE_LABEL = { 'tauri-mobile': 'Phone app', 'tauri-desktop': 'Desktop app', web: 'Web' } as const;
/** UNI-200: the Unicrab — floating crab (phone), over other apps (Android overlay), mascot window (desktop). */
function MascotSettings() {
  const bridge = overlayBridge();
  const [overlayMsg, setOverlayMsg] = createSignal<string | undefined>();
  const [overlayChecked, setOverlayChecked] = createSignal(overlayRunning());
  const [windowOn, setWindowOn] = createSignal(mascotWindowOn());
  const [windowSize, setWindowSizeSig] = createSignal<MascotWindowSize>(mascotWindowSize());
  const pickSize = (size: MascotWindowSize) => {
    setMascotWindowSize(size);
    setWindowSizeSig(size);
    if (windowOn()) void syncMascotWindow(true);
  };
  // Back from the system "Display over other apps" page: finish turning it on.
  onMount(() => {
    const onVisible = () => {
      if (document.visibilityState !== 'visible' || overlayMsg() !== 'pending') return;
      if (bridge?.canDraw?.()) {
        setOverlay(true);
        setOverlayChecked(true);
        setOverlayMsg(undefined);
      } else setOverlayMsg('denied');
    };
    document.addEventListener('visibilitychange', onVisible);
    onCleanup(() => document.removeEventListener('visibilitychange', onVisible));
  });
  const toggleOverlay = (on: boolean) => {
    const step = setOverlay(on);
    setOverlayChecked(step === 'on');
    setOverlayMsg(step === 'needs-permission' ? 'pending' : undefined);
  };
  const phone = () => !desktopShell() && can('hosts');
  return (
    <Show when={phone() || !!bridge?.start || can('tray')}>
      <Section title="Unicrab" icon={<Bell size={14} />}>
        <div class="mascot-settings-hero" aria-hidden="true">
          <Unicrab state="idle" size={64} />
          <p class="text-xs text-text-dim">Unicrab shows what your agents are doing and pops up their replies. Tap it for everything that needs you.</p>
        </div>
        <Show when={phone()}>
          <Row label="Floating crab" description="A button you can drag to any edge; long-press for quick actions">
            <SwitchField label="Floating crab" checked={floatingMascotOn()} onChange={setFloatingMascotOn} />
          </Row>
        </Show>
        <Show when={bridge?.start}>
          <Row
            label="Over other apps"
            description={
              overlayMsg() === 'pending'
                ? 'Allow "Display over other apps" for UniPi, then come back'
                : overlayMsg() === 'denied'
                  ? 'Not allowed yet: UniPi needs "Display over other apps"'
                  : 'Keeps the crab and its replies on screen while you use other apps (a small ongoing notification shows it is on)'
            }
          >
            <SwitchField label="Over other apps" checked={overlayChecked()} onChange={toggleOverlay} />
          </Row>
        </Show>
        <Show when={can('tray')}>
          <Row label="Mascot window" description="An always-on-top crab with reply bubbles; also in the tray menu">
            <SwitchField label="Mascot window" checked={windowOn()} onChange={(v) => (setWindowOn(v), void syncMascotWindow(v))} />
          </Row>
          <Show when={windowOn()}>
          <Row label="Crab size" description="Only the crab floats on your desktop (no background); drag it anywhere">
            <div class="mascot-size-options" role="radiogroup" aria-label="Crab size" data-testid="mascot-size">
              {MASCOT_WINDOW_SIZES.map((size, i) => (
                <button type="button" role="radio" aria-checked={windowSize() === size} onClick={() => pickSize(size)}>
                  {['Small', 'Medium', 'Large'][i]}
                </button>
              ))}
            </div>
          </Row>
          </Show>
        </Show>
      </Section>
    </Show>
  );
}

function modeHint(): string | undefined {
  const mode = platform().mode;
  if (mode === 'web') return 'Running in a browser: only the board works here. Terminals, chat and files need the UniPi app.';
  if (mode === 'tauri-desktop') return 'Phone-only features (QR scan, haptics, push notifications) are hidden.';
  return undefined;
}

const AppInfoSchema = z.object({ version: z.string(), platform: z.string(), proto: z.string() });

function About() {
  const [info] = createResource(async () => call('app_info', {}, AppInfoSchema));
  return (
    <Section title="About" icon={<Info size={14} />}>
      <Requires cap="updater" fallback={null}>
        <Show when={platform().os === 'ios'} fallback={<UpdateCard />}>
          <IosUpdateCard />
        </Show>
      </Requires>
      <Requires cap="desktopUpdater" fallback={null}>
        <DesktopUpdateCard />
      </Requires>
      <Show when={platform().mode !== 'tauri-mobile'}>
        <Row label="Mode" description={modeHint()}>
          <span class="text-sm text-text-muted" data-testid="platform-mode">{MODE_LABEL[platform().mode]}</span>
        </Row>
      </Show>
      <Row label="Version">
        <span class="text-sm text-text-muted">
          <Show when={info()?.ok} fallback="unavailable">
            {info()!.ok ? (info() as { ok: true; value: z.infer<typeof AppInfoSchema> }).value.version : ''}
          </Show>
        </span>
      </Row>
      <Row label="Platform">
        <span class="text-sm text-text-muted">
          <Show when={info()?.ok} fallback="unavailable">
            {info()!.ok ? (info() as { ok: true; value: z.infer<typeof AppInfoSchema> }).value.platform : ''}
          </Show>
        </span>
      </Row>
      <Row label="Protocol">
        <span class="text-sm text-text-muted">
          <Show when={info()?.ok} fallback="unavailable">
            {info()!.ok ? (info() as { ok: true; value: z.infer<typeof AppInfoSchema> }).value.proto : ''}
          </Show>
        </span>
      </Row>
      <Row label="Licences" description="Inter and JetBrains Mono fonts: SIL Open Font License · Lucide icons: ISC">
        <span class="text-sm text-text-muted">MIT</span>
      </Row>
    </Section>
  );
}

export default function Settings() {
  const [settings, update] = useSettings();
  const api = useSettingsApi();
  const hostsCtx = useHosts();
  const hostsList = () => hostsCtx.hosts().map((h) => ({ id: h.id, name: h.name }));
  const [relayUrl, setRelayUrl] = createSignal(settings.connection.relay.mode === 'custom' ? settings.connection.relay.url : '');

  const [desktopOn, setDesktopOn] = createSignal(desktopNotifyOn());

  function onRelayUrlChange(v: string) {
    setRelayUrl(v);
    update('connection.relay', { mode: 'custom', url: v });
  }

  // Desktop: the section list on the left (read from the rendered sections, so gated ones that
  // render nothing don't appear), the picked section on the right.
  const [active, setActive] = createSignal<string | undefined>();
  const [titles, setTitles] = createSignal<string[]>([]);
  let page: HTMLDivElement | undefined;
  onMount(() => {
    const read = () => setTitles([...(page?.querySelectorAll<HTMLElement>('section[data-section]') ?? [])].map((el) => el.dataset.section!));
    read();
    const mo = new MutationObserver(read);
    if (page) mo.observe(page, { childList: true, subtree: true });
    onCleanup(() => mo.disconnect());
  });
  const current = () => (active() && titles().includes(active()!) ? active() : titles()[0]);
  const onNavKey = (e: KeyboardEvent) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const list = titles();
    const at = list.indexOf(current() ?? '');
    const next = list[Math.max(0, Math.min(list.length - 1, at + (e.key === 'ArrowDown' ? 1 : -1)))];
    setActive(next);
    (e.currentTarget as HTMLElement).querySelector<HTMLElement>(`[data-nav="${next}"]`)?.focus();
  };

  return (
    <SectionNav.Provider value={{ active: current }}>
    <div class="settings-page" ref={page}>
      <Show when={desktopShell()}>
        <nav class="settings-nav" aria-label="Settings sections" onKeyDown={onNavKey}>
          <For each={titles()}>
            {(title) => (
              <button type="button" class="settings-nav-item" data-nav={title} aria-current={current() === title ? 'page' : undefined} onClick={() => setActive(title)}>
                {title}
              </button>
            )}
          </For>
        </nav>
      </Show>
      <Show when={can('nativeNotifications')}>
        <Section title="Desktop" icon={<Bell size={14} />}>
          <Row label="Desktop notifications" description="A system notification when pi needs you or finishes, while UniPi isn't in front">
            <SwitchField label="Desktop notifications" checked={desktopOn()} onChange={(v) => (setDesktopNotifyOn(v), setDesktopOn(v))} />
          </Row>
        </Section>
      </Show>
      <SharingSettings Section={Section} />
      <Requires cap="hosts" fallback={null}>
        <PushAccess hosts={hostsList()} Section={Section} />
      </Requires>
      <MascotSettings />
      <Section title="Hosts" icon={<Server size={14} />}>
        <Requires cap="hosts" variant="inline">
          <HostsSettings />
        </Requires>
      </Section>

      <Requires cap="terminal" fallback={null}>
      <Section title="Terminal" icon={<MonitorSmartphone size={14} />}>
        <Row label="Font size">
          <Stepper
            label="Font size"
            value={settings.terminal.fontSize}
            min={8}
            max={32}
            step={1}
            onChange={(v) => update('terminal.fontSize', v)}
          />
        </Row>
        <Row label="Line height">
          <Stepper
            label="Line height"
            value={settings.terminal.lineHeight}
            min={1}
            max={2}
            step={0.1}
            format={(v) => v.toFixed(1)}
            onChange={(v) => update('terminal.lineHeight', v)}
          />
        </Row>
        <Row label="Cursor style">
          <Segmented
            label="Cursor style"
            value={settings.terminal.cursorStyle}
            options={['block', 'underline', 'bar'] as const}
            onChange={(v) => update('terminal.cursorStyle', v)}
          />
        </Row>
        <Row label="Cursor blink">
          <SwitchField label="Cursor blink" checked={settings.terminal.cursorBlink} onChange={(v) => update('terminal.cursorBlink', v)} />
        </Row>
        <Row label="Scrollback">
          <Stepper
            label="Scrollback"
            value={settings.terminal.scrollback}
            min={0}
            max={100000}
            step={1000}
            onChange={(v) => update('terminal.scrollback', v)}
          />
        </Row>
        <Row label="Phone-size policy" description="Fit phone reflows the session; Keep PC size leaves it alone">
          <Segmented
            label="Phone-size policy"
            value={settings.terminal.sizePolicy}
            options={['fit_phone', 'keep_pc'] as const}
            labels={{ fit_phone: 'Fit phone', keep_pc: 'Keep PC size' }}
            onChange={(v) => update('terminal.sizePolicy', v)}
          />
        </Row>
        <Row label="Keep screen on">
          <SwitchField label="Keep screen on" checked={settings.terminal.keepScreenOn} onChange={(v) => update('terminal.keepScreenOn', v)} />
        </Row>
        <Row label="Reopen last session">
          <SwitchField label="Reopen last session" checked={settings.terminal.reopenLast} onChange={(v) => update('terminal.reopenLast', v)} />
        </Row>
      </Section>
      </Requires>

      <Requires cap="terminal" fallback={null}>
      <Section title="Input" icon={<Keyboard size={14} />}>
        <Requires cap="haptics" fallback={null}>
          <Row label="Haptics">
            <SwitchField label="Haptics" checked={settings.input.haptics} onChange={(v) => update('input.haptics', v)} />
          </Row>
        </Requires>
        <Row label="Composer autocorrect">
          <SwitchField label="Composer autocorrect" checked={settings.input.composerAutocorrect} onChange={(v) => update('input.composerAutocorrect', v)} />
        </Row>
        <Row label="Use icons for modifier keys" description="^ ⇧ ↺ ⇥ ⌥ instead of Ctrl Shift Esc Tab Opt">
          <SwitchField label="Use icons for modifier keys" checked={settings.input.modifierIcons} onChange={(v) => update('input.modifierIcons', v)} />
        </Row>
        <ToolbarEditor />
      </Section>
      </Requires>

      <Show when={desktopShell()}>
        <Section title="Keyboard shortcuts" icon={<Keyboard size={14} />}>
          <ShortcutsSettings />
        </Section>
      </Show>

      <Requires cap="files" fallback={null}>
      <Section title="Editor" icon={<Type size={14} />}>
        <Row label="Vim mode" description="vim keybindings in the file editor, with a NORMAL/INSERT/VISUAL indicator and :w to save">
          <SwitchField label="Vim mode" checked={settings.editor.vim} onChange={(v) => update('editor.vim', v)} />
        </Row>
      </Section>
      </Requires>

      <Requires cap="hosts" fallback={null}>
      <Section title="Connection" icon={<Cpu size={14} />}>
        <Row label="Relay">
          <Segmented
            label="Relay"
            value={settings.connection.relay.mode}
            options={['n0', 'custom'] as const}
            labels={{ n0: 'n0 (default)', custom: 'Custom' }}
            onChange={(v) => update('connection.relay', v === 'n0' ? { mode: 'n0' } : { mode: 'custom', url: relayUrl() || 'https://' })}
          />
        </Row>
        <Show when={settings.connection.relay.mode === 'custom'}>
          <div class="px-4 py-3">
            <label class="text-xs text-text-dim block mb-1" for="relay-url">
              Relay URL
            </label>
            <input
              id="relay-url"
              type="url"
              class="w-full rounded-input bg-surface-2 border border-line px-3 py-2 text-sm text-text"
              value={relayUrl()}
              onInput={(e) => onRelayUrlChange(e.currentTarget.value)}
              placeholder="https://relay.example.com"
            />
            <Show when={api.error()?.path?.[0] === 'connection' && api.error()?.path?.[1] === 'relay'}>
              <p class="text-xs text-shell mt-1">{api.error()?.message}</p>
            </Show>
          </div>
        </Show>
        <Row label="Path preference">
          <Segmented
            label="Path preference"
            value={settings.connection.pathPreference}
            options={['auto', 'prefer_ssh'] as const}
            labels={{ auto: 'Auto', prefer_ssh: 'Prefer SSH' }}
            onChange={(v) => update('connection.pathPreference', v)}
          />
        </Row>
      </Section>
      </Requires>

      <Section title="Board" icon={<FolderKanban size={14} />}>
        <Row label="Default project">
          <input
            type="text"
            class="rounded-input bg-surface-2 border border-line px-3 py-1.5 text-sm text-text w-36"
            value={settings.board.defaultProject ?? ''}
            placeholder="none"
            onChange={(e) => update('board.defaultProject', e.currentTarget.value.trim() === '' ? null : e.currentTarget.value.trim())}
          />
        </Row>
        <Row label="Show backlog">
          <SwitchField label="Show backlog" checked={settings.board.showBacklog} onChange={(v) => update('board.showBacklog', v)} />
        </Row>
      </Section>

      <Requires cap="notifications" fallback={null}>
        <NotifySettings />
      </Requires>

      <About />
    </div>
    </SectionNav.Provider>
  );
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
