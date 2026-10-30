/**
 * Settings → "Sharing with phones" and "This computer" (desktop app, UNI-248),
 * plus the owner's request dialog (`SharingRequestsRunner`).
 *
 * The switches write through the PC's own `unipi-host` CLI (`sharing_change`),
 * so they and the commands are one code path; serve re-reads config.json per
 * request. "Keep running after logout" checks systemd linger and offers the
 * polkit-prompted `pkexec loginctl enable-linger $USER` (via `unipi-host
 * linger enable`) — the one step that needs root.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { A } from '@solidjs/router';
import { Clock, Globe, Hand, HardDrive, Monitor, MonitorSmartphone, Plug, Power, QrCode, RefreshCw, Send, ShieldCheck, Smartphone, Terminal } from 'lucide-solid';
import { MODE_LABEL, refreshThisComputer, revokeDevice, seenText, thisComputer } from '../lib/thisComputer';
import type { DeviceItem } from '@unipi/protocol';
import { z } from 'zod';
import type { AppError } from '@unipi/protocol';
import { SwitchField } from './Switch';
import { BoardSheet } from './board/BoardSheet';
import { HostSupervisorCard, hostLog, restartHost } from './HostSupervisorCard';
import { ComponentsCard } from './ComponentsCard';
import { CronSettingsCard } from './CronSettings';
import { setupBannerVisible } from '../lib/setupGate';
import { useAutostart } from './LifecycleUi';
import { call } from '../lib/ipc';
import { can } from '../lib/platform';
import { nativeNotify } from '../lib/desktop';
import { browserName } from '../lib/browser';
import { openPairPhone } from '../lib/pairPhone';
import {
  ago,
  changeSharing,
  devicesLine,
  FEATURE_HELP,
  FEATURE_LABEL,
  featureOn,
  INSTALL_COMMAND,
  OWNER_POLL_MS,
  parsePorts,
  pendingRequests,
  refreshSharing,
  requestTitle,
  sharingStatus,
  type ApproveFor,
  type FeatureKind,
  type RequestView,
  type SharingChange,
  type SharingStatus,
} from '../lib/sharing';
import './sharing.css';

const ICON: Record<FeatureKind, typeof Plug> = { forward: Plug, browser: Globe, desktop: MonitorSmartphone, push: Send, cron: Clock };
/** The switches shown; desktop streaming is deferred (D-15), so it has none. */
const FEATURES: FeatureKind[] = ['forward', 'browser', 'push', 'cron'];

function useChange() {
  const [busy, setBusy] = createSignal<string | undefined>();
  const [error, setError] = createSignal<AppError | undefined>();
  async function run(key: string, change: SharingChange) {
    setBusy(key);
    setError(undefined);
    const r = await changeSharing(change);
    setBusy(undefined);
    if (!r.ok) setError(r.error);
    return r.ok;
  }
  return { busy, error, run, setError };
}

function ErrorLine(props: { error: AppError | undefined }) {
  return (
    <Show when={props.error}>
      {(e) => (
        <p class="share-empty preview-field-error" role="alert">
          {e().message}
          <Show when={e().hint}> — {e().hint}</Show>
        </p>
      )}
    </Show>
  );
}

function FeatureRow(props: { feature: FeatureKind; s: SharingStatus; change: ReturnType<typeof useChange> }) {
  const Icon = ICON[props.feature];
  const on = () => featureOn(props.s, props.feature);
  const key = () => props.feature;
  const busy = () => props.change.busy() === key();
  const [ports, setPorts] = createSignal(props.s.forward.ports.join(', '));
  const [portsError, setPortsError] = createSignal<string | undefined>();
  /** "Only these ports" picked while any port is still saved: show the box until Save. */
  const [editingPorts, setEditingPorts] = createSignal(false);
  const [path, setPath] = createSignal(props.s.browser.path ?? '');
  createEffect(() => setPorts(props.s.forward.ports.join(', ')));
  createEffect(() => setPath(props.s.browser.path ?? ''));

  const enableChange = (): SharingChange => {
    switch (props.feature) {
      case 'forward':
        return props.s.forward.ports.length && !props.s.forward.any ? { kind: 'forward_enable', any: false, ports: props.s.forward.ports } : { kind: 'forward_enable', any: true };
      case 'browser':
        return { kind: 'browser_enable', headed: !props.s.browser.headless };
      case 'desktop':
        return { kind: 'desktop_enable', view_only: props.s.desktop.view_only };
      case 'push':
        return { kind: 'push_enable' };
      case 'cron':
        return { kind: 'cron_enable' };
    }
  };
  const devices = () => {
    const s = props.s;
    return props.feature === 'forward'
      ? s.forward.devices
      : props.feature === 'browser'
        ? s.browser.devices
        : props.feature === 'desktop'
          ? s.desktop.devices
          : props.feature === 'push'
            ? s.push.devices
            : s.cron?.devices ?? [];
  };
  const savePorts = () => {
    const parsed = parsePorts(ports());
    if (!parsed.ok) return setPortsError(parsed.message);
    setPortsError(undefined);
    void props.change.run(key(), { kind: 'forward_enable', any: false, ports: parsed.ports }).then((ok) => ok && setEditingPorts(false));
  };

  return (
    <div data-testid={`share-row-${props.feature}`} data-on={on() ? 'true' : 'false'}>
      <div class="settings-row">
        <span class="settings-icon"><Icon size={18} /></span>
        <div class="min-w-0">
          <p class="text-[15px] text-text">{FEATURE_LABEL[props.feature]}</p>
          <p class="text-xs text-text-dim mt-0.5">{FEATURE_HELP[props.feature]}</p>
        </div>
        <div class="shrink-0 text-right text-text-muted">
          <Show when={busy()} fallback={<SwitchField label={FEATURE_LABEL[props.feature]} checked={on()} onChange={(v) => void props.change.run(key(), v ? enableChange() : { kind: 'disable', feature: props.feature })} />}>
            <RefreshCw size={18} class="refresh-spinning" />
          </Show>
        </div>
      </div>
      <Show when={on()}>
        <div class="share-row-detail">
          <Show when={props.feature === 'forward'}>
            <div class="seg share-seg" role="radiogroup" aria-label="Which ports">
              <button type="button" role="radio" class="seg-option" aria-checked={props.s.forward.any && !editingPorts()} disabled={busy()} onClick={() => { setEditingPorts(false); setPortsError(undefined); if (!props.s.forward.any) void props.change.run(key(), { kind: 'forward_enable', any: true }); }}>
                Any port
              </button>
              <button
                type="button"
                role="radio"
                class="seg-option"
                aria-checked={!props.s.forward.any || editingPorts()}
                disabled={busy()}
                onClick={() => {
                  if (!props.s.forward.any) return;
                  if (props.s.forward.ports.length) void props.change.run(key(), { kind: 'forward_enable', any: false, ports: props.s.forward.ports });
                  else {
                    setEditingPorts(true);
                    setPortsError('Type the ports to allow, then Save');
                  }
                }}
              >
                Only these ports
              </button>
            </div>
            <Show when={!props.s.forward.any || editingPorts()}>
              <form
                class="preview-manual-row"
                onSubmit={(e) => {
                  e.preventDefault();
                  savePorts();
                }}
              >
                <input class="preview-input" inputmode="numeric" placeholder="5173, 8080" aria-label="Allowed ports" value={ports()} aria-invalid={portsError() ? 'true' : undefined} onInput={(e) => (setPorts(e.currentTarget.value), setPortsError(undefined))} />
                <button type="submit" class="btn btn-secondary" disabled={busy()}>Save</button>
              </form>
              <Show when={portsError()}>
                <p class="preview-field-error" role="alert">{portsError()}</p>
              </Show>
            </Show>
          </Show>
          <Show when={props.feature === 'browser'}>
            <label class="share-check" data-testid="share-browser-window">
              <input type="checkbox" checked={!props.s.browser.headless} disabled={busy()} onChange={(e) => void props.change.run(key(), { kind: 'browser_enable', headed: e.currentTarget.checked })} /> Show its window on this PC (otherwise only in the app)
            </label>
            <p class="preview-muted">
              {props.s.browser.headless
                ? 'Off: it runs without a window. On: you see the same tabs the phone drives — a running browser switches within a few seconds.'
                : 'Its window shows on this PC with the tabs the phone drives. A separate Chrome with its own profile — not your everyday browser.'}
            </p>
            <p class="preview-muted">
              {props.s.browser.found ? `Uses ${browserName(props.s.browser.found.name)} (${props.s.browser.found.path})` : 'Google Chrome not found — install Google Chrome on this PC, or set its path:'} · quits after{' '}
              {props.s.browser.idle_minutes ? `${props.s.browser.idle_minutes} min` : 'never'} with nobody watching
            </p>
            <form
              class="preview-manual-row"
              onSubmit={(e) => {
                e.preventDefault();
                void props.change.run(key(), { kind: 'browser_enable', headed: !props.s.browser.headless, path: path().trim() });
              }}
            >
              <input class="preview-input" placeholder="Browser path (empty = find one)" aria-label="Browser path" value={path()} onInput={(e) => setPath(e.currentTarget.value)} />
              <button type="submit" class="btn btn-secondary" disabled={busy() || path().trim() === (props.s.browser.path ?? '')}>Save</button>
            </form>
          </Show>
          <Show when={props.feature === 'desktop'}>
            <label class="share-check">
              <input type="checkbox" checked={props.s.desktop.view_only} disabled={busy()} onChange={(e) => void props.change.run(key(), { kind: 'desktop_enable', view_only: e.currentTarget.checked })} /> Watch only (phones can't use the mouse or keyboard)
            </label>
            <Show when={props.s.desktop.reason}>
              <p class="preview-muted">Can't stream right now: {props.s.desktop.reason?.replace(/_/g, ' ')}.</p>
            </Show>
          </Show>
          <p class="preview-muted">{devicesLine(props.s, devices())}</p>
        </div>
      </Show>
    </div>
  );
}

/** The pending requests list with Approve (time limit) / Deny. */
export function RequestList(props: { requests: RequestView[]; compact?: boolean; onDone?: () => void }) {
  const change = useChange();
  return (
    <>
      <For each={props.requests}>
        {(r) => <RequestItem r={r} change={change} onDone={props.onDone} />}
      </For>
      <ErrorLine error={change.error()} />
    </>
  );
}

const FOR_OPTIONS: Array<{ value: ApproveFor | undefined; label: string }> = [
  { value: '1h', label: 'For 1 hour' },
  { value: '8h', label: 'For 8 hours' },
  { value: undefined, label: 'Until I turn it off' },
];

function RequestItem(props: { r: RequestView; change: ReturnType<typeof useChange>; onDone?: () => void }) {
  const [forWhat, setFor] = createSignal<ApproveFor | undefined>('1h');
  const busy = () => props.change.busy() === props.r.id;
  const decide = async (approve: boolean) => {
    const ok = await props.change.run(props.r.id, approve ? { kind: 'approve', id: props.r.id, for: forWhat() } : { kind: 'deny', id: props.r.id });
    if (ok) props.onDone?.();
  };
  return (
    <div class="share-request" data-testid={`share-request-${props.r.id}`}>
      <div>
        <p class="share-request-title">{requestTitle(props.r)}</p>
        <p class="preview-muted">{ago(props.r.created_at)} · nothing changes until you answer</p>
      </div>
      <Show when={props.r.note}>
        <p class="share-note-quote">“{props.r.note}”</p>
      </Show>
      <div class="share-for" role="radiogroup" aria-label="How long">
        <For each={FOR_OPTIONS}>
          {(o) => (
            <button type="button" role="radio" aria-checked={forWhat() === o.value} disabled={busy()} onClick={() => setFor(o.value)}>
              {o.label}
            </button>
          )}
        </For>
      </div>
      <div class="share-request-actions">
        <button type="button" class="btn btn-primary" disabled={busy()} data-testid="share-approve" onClick={() => void decide(true)}>
          <ShieldCheck size={16} /> Allow
        </button>
        <button type="button" class="btn btn-secondary" disabled={busy()} data-testid="share-deny" onClick={() => void decide(false)}>
          <Hand size={16} /> Deny
        </button>
      </div>
    </div>
  );
}

function Group(props: { title: string; icon: JSX.Element; children: JSX.Element; testId?: string }) {
  return (
    <section class="mb-7" data-section={props.title} data-testid={props.testId}>
      <h2 class="section-label settings-section-label">
        {props.icon}
        {props.title}
      </h2>
      <div class="settings-group rounded-card bg-surface-1">{props.children}</div>
    </section>
  );
}

/** Both sections; renders nothing outside the desktop app. `Section` is Settings' own wrapper (hides off-screen sections on the desktop shell). */
export function SharingSettings(props: { Section?: (p: { title: string; icon: JSX.Element; children: JSX.Element }) => JSX.Element }) {
  const Section = props.Section ?? Group;
  const change = useChange();
  onMount(() => void refreshSharing());
  const status = () => sharingStatus();
  const s = () => {
    const r = status();
    return r && r.ok ? r.value : undefined;
  };
  const failed = () => {
    const r = status();
    return r && !r.ok ? r.error : undefined;
  };
  return (
    <Show when={can('localSharing')}>
      <Section title="Sharing with phones" icon={<MonitorSmartphone size={14} />}>
        <div data-testid="share-settings">
          <Show when={!status()}>
            <div class="share-empty"><RefreshCw size={14} class="refresh-spinning inline" /> Reading this PC's settings…</div>
          </Show>
          <Show when={failed()}>
            {(e) => (
              <div class="share-row-detail" style={{ 'padding-left': '16px', 'padding-top': '14px' }}>
                <p class="text-[15px] text-text">{e().message}</p>
                <Show when={e().code === 'E_NOT_FOUND' || e().code === 'E_UNSUPPORTED'}>
                  <p class="preview-muted">Install or update unipi-host on this PC:</p>
                  <code class="preview-muted">{INSTALL_COMMAND}</code>
                </Show>
                <button type="button" class="btn btn-secondary" style={{ 'align-self': 'flex-start' }} onClick={() => void refreshSharing()}>
                  <RefreshCw size={16} /> Try again
                </button>
              </div>
            )}
          </Show>
          <Show when={s()}>
            {(st) => (
              <>
                <Show when={pendingRequests(st()).length > 0}>
                  <p class="share-empty" style={{ 'padding-bottom': '0' }}>
                    <Clock size={14} class="inline" /> Waiting for your answer
                  </p>
                  <RequestList requests={pendingRequests(st())} />
                </Show>
                <For each={FEATURES}>{(f) => <FeatureRow feature={f} s={st()} change={change} />}</For>
                <ErrorLine error={change.error()} />
              </>
            )}
          </Show>
        </div>
      </Section>
      <Show when={s()}>
        {(st) => <ThisComputer s={st()} Section={Section} />}
      </Show>
    </Show>
  );
}

/** UNI-233: the paired phones, each with Unpair (asks first). */
function PhonesList() {
  const [confirm, setConfirm] = createSignal<DeviceItem | undefined>();
  const [busy, setBusy] = createSignal(false);
  const [err, setErr] = createSignal<string | undefined>();
  const tc = thisComputer;
  const devices = () => {
    const t = tc();
    return t.kind === 'ready' ? t.devices : [];
  };
  const unpair = async () => {
    const d = confirm();
    if (!d) return;
    setBusy(true);
    setErr(undefined);
    try {
      await revokeDevice(d.id);
      setConfirm(undefined);
      void refreshSharing();
    } catch (e) {
      setErr((e as { message?: string })?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Show when={tc().kind === 'ready'}>
      <div data-testid="tc-phones">
        <p class="share-empty" style={{ 'padding-bottom': '2px' }}>
          {devices().length ? `Phones that can reach this PC (${devices().length})` : 'No phone paired yet — use Pair a phone above.'}
        </p>
        <For each={devices()}>
          {(d) => (
            <div class="settings-row" data-testid="tc-phone">
              <span class="settings-icon"><Smartphone size={18} /></span>
              <div class="min-w-0">
                <p class="text-[15px] text-text truncate" title={d.name}>{d.name}</p>
                <p class="text-xs text-text-dim mt-0.5">{seenText(d)} · {d.id.slice(0, 8)}</p>
              </div>
              <div class="shrink-0">
                <button type="button" class="btn btn-ghost btn-sm" data-testid="tc-unpair" onClick={() => (setErr(undefined), setConfirm(d))}>
                  Unpair
                </button>
              </div>
            </div>
          )}
        </For>
      </div>
      <BoardSheet open={!!confirm()} onClose={() => !busy() && setConfirm(undefined)} label={`Unpair ${confirm()?.name ?? 'this phone'}?`} testId="tc-unpair-sheet">
        <p class="preview-muted" style={{ margin: '0 0 12px' }}>
          It can't reach this PC any more — no terminals, chats or files — until you pair it again with a new QR code.
        </p>
        <Show when={err()}>
          <p class="preview-field-error" role="alert">{err()}</p>
        </Show>
        <div class="share-request-actions">
          <button type="button" class="btn btn-danger" disabled={busy()} data-testid="tc-unpair-confirm" onClick={() => void unpair()}>
            {busy() ? 'Unpairing…' : 'Unpair'}
          </button>
          <button type="button" class="btn btn-secondary" disabled={busy()} onClick={() => setConfirm(undefined)}>
            Keep
          </button>
        </div>
      </BoardSheet>
    </Show>
  );
}

function ThisComputer(props: { s: SharingStatus; Section: (p: { title: string; icon: JSX.Element; children: JSX.Element }) => JSX.Element }) {
  const change = useChange();
  const Section = props.Section;
  const [restarting, setRestarting] = createSignal(false);
  const restart = async () => {
    setRestarting(true);
    try {
      await restartHost();
      void refreshThisComputer();
    } catch (e) {
      const message = (e as { message?: string })?.message ?? String(e);
      change.setError({ code: 'E_INTERNAL', message, retryable: true });
    } finally {
      setRestarting(false);
    }
  };
  onMount(() => void refreshThisComputer());
  const runBy = () => {
    const t = thisComputer();
    return t.kind === 'ready' ? `Run by ${MODE_LABEL[t.info.mode]}` : undefined;
  };
  return (
    <Section title="This computer" icon={<HardDrive size={14} />}>
      <Show when={setupBannerVisible()}>
        <div class="setup-banner" data-testid="setup-banner">
          <p>This PC isn't set up for UniPi yet.</p>
          <A href="/setup" class="btn btn-primary btn-sm">Set up this PC</A>
        </div>
      </Show>
      <div data-testid="share-this-computer">
        <div class="settings-row">
          <span class="settings-icon"><QrCode size={18} /></span>
          <div class="min-w-0">
            <p class="text-[15px] text-text">Pair a phone</p>
            <p class="text-xs text-text-dim mt-0.5">Shows a QR code to scan with UniPi on the phone (Add a PC → Pair with QR)</p>
          </div>
          <div class="shrink-0">
            <button type="button" class="btn btn-primary" data-testid="pair-phone-open" onClick={() => openPairPhone()}>
              Pair a phone
            </button>
          </div>
        </div>
        <div class="settings-row">
          <span class="settings-icon"><Monitor size={18} /></span>
          <div class="min-w-0">
            <p class="text-[15px] text-text">unipi-host {props.s.version}</p>
            <p class="text-xs text-text-dim mt-0.5" data-testid="tc-host-line">
              {runBy() ? `${runBy()} · ` : ''}
              {props.s.host_dir} · {props.s.devices.length} paired device{props.s.devices.length === 1 ? '' : 's'}
            </p>
            <Show when={props.s.update?.available}>
              <p class="text-xs mt-0.5 board-accent" data-testid="host-update-available">
                Version {props.s.update!.latest} is available. Updating restarts the host: phones reconnect by themselves.
              </p>
            </Show>
            <Show when={!props.s.update?.available && props.s.update?.error}>
              <p class="text-xs text-text-dim mt-0.5">Couldn't check for updates: {props.s.update!.error}</p>
            </Show>
          </div>
          <div class="shrink-0 flex items-center gap-2">
            <button type="button" class="btn btn-ghost btn-sm" data-testid="host-restart" disabled={!!restarting() || !!change.busy()} title="Restart this PC's host" onClick={() => void restart()}>
              <Show when={restarting()} fallback={<RefreshCw size={16} />}>
                <RefreshCw size={16} class="refresh-spinning" />
              </Show>
            </button>
            <Show when={props.s.update?.available}>
              <button type="button" class="btn btn-primary" data-testid="host-update" disabled={!!change.busy()} onClick={() => void change.run('host_update', { kind: 'host_update' })}>
                <Show when={change.busy() === 'host_update'} fallback={'Update'}>
                  <RefreshCw size={16} class="refresh-spinning" /> Updating…
                </Show>
              </button>
            </Show>
          </div>
        </div>
        <PhonesList />
        <HostSupervisorCard />
        <AutostartRow />
        <Show when={props.s.update}>
          <div class="settings-row">
            <span class="settings-icon"><RefreshCw size={18} /></span>
            <div class="min-w-0">
              <p class="text-[15px] text-text">Update the host by itself</p>
              <p class="text-xs text-text-dim mt-0.5">
                {props.s.update!.mode === 'auto'
                  ? 'On — a new unipi-host is installed as soon as it is out, then the host restarts (phones reconnect)'
                  : 'Off — you are asked here first (the default)'}
              </p>
            </div>
            <div class="shrink-0">
              <SwitchField
                label="Update the host by itself"
                checked={props.s.update!.mode === 'auto'}
                disabled={!!change.busy()}
                onChange={(on) => void change.run('host_update_mode', { kind: 'host_update_mode', auto: on })}
                testId="host-update-auto"
              />
            </div>
          </div>
        </Show>
        <HostLogRow busy={!!change.busy()} />
        <Show when={can('selfInstall')}>
          <ComponentsCard />
        </Show>
        <CronSettingsCard />
        <ErrorLine error={change.error()} />
      </div>
    </Section>
  );
}

/** UNI-235: the host's own log — a titled row like every other, with the text
 *  behind a disclosure so it never steals space from the settings above it. */
function HostLogRow(props: { busy: boolean }) {
  const [open, setOpen] = createSignal(false);
  const [lines, setLines] = createSignal<string[]>([]);
  const [loading, setLoading] = createSignal(false);
  const toggle = async () => {
    if (open()) {
      setOpen(false);
      return;
    }
    setLoading(true);
    setLines(await hostLog());
    setLoading(false);
    setOpen(true);
  };
  return (
    <div class="settings-row">
      <span class="settings-icon"><Terminal size={18} /></span>
      <div class="min-w-0">
        <p class="text-[15px] text-text">Host log</p>
        <p class="text-xs text-text-dim mt-0.5">What this PC's host printed — the last lines of host.log, for when something stops.</p>
        <Show when={open()}>
          <pre class="preview-command" data-testid="host-log" style={{ 'max-height': '160px', overflow: 'auto' }}>
            {lines().slice(-40).join('\n') || '(empty)'}
          </pre>
        </Show>
      </div>
      <div class="shrink-0">
        <button type="button" class="btn btn-secondary btn-sm" data-testid="host-log-open" disabled={props.busy || loading()} onClick={() => void toggle()}>
          <Show when={loading()} fallback={open() ? 'Hide' : 'Show'}>
            <RefreshCw size={16} class="refresh-spinning" />
          </Show>
        </button>
      </div>
    </div>
  );
}

// ------------------------------------------------------------- owner dialog

const PendingSchema = z.array(z.object({ id: z.string(), feature: z.enum(['forward', 'browser', 'desktop', 'push']), device_name: z.string(), device_id: z.string().default(''), status: z.literal('pending'), created_at: z.number(), note: z.string().optional(), scope: z.record(z.string(), z.unknown()).default({}) }).passthrough());

/**
 * Desktop app on the PC: polls this PC's pending requests; a new one opens
 * the "<Pixel> asks for desktop streaming" dialog (and a system notification
 * when UniPi isn't in front). Each request is announced once.
 */
export function SharingRequestsRunner() {
  if (!can('localSharing')) return null;
  const [open, setOpen] = createSignal<RequestView[]>([]);
  const seen = new Set<string>();
  const tick = async () => {
    const r = await call('local_requests', undefined, PendingSchema);
    if (!r.ok) return;
    const pending = r.value as unknown as RequestView[];
    const fresh = pending.filter((p) => !seen.has(p.id));
    for (const p of pending) seen.add(p.id);
    // Still-pending ones stay in the dialog; answered elsewhere (CLI) drop out.
    setOpen((list) => list.filter((x) => pending.some((p) => p.id === x.id)));
    if (fresh.length) {
      setOpen((list) => [...list, ...fresh.filter((f) => !list.some((x) => x.id === f.id))]);
      if (!document.hasFocus()) for (const f of fresh.slice(0, 3)) void nativeNotify('UniPi — a phone is asking', requestTitle(f));
      void refreshSharing();
    }
  };
  void tick();
  const timer = setInterval(() => void tick(), OWNER_POLL_MS);
  onCleanup(() => clearInterval(timer));
  return (
    <BoardSheet open={open().length > 0} onClose={() => setOpen([])} label={open().length > 1 ? `${open().length} requests from phones` : 'A phone is asking'} testId="share-request-sheet">
      <p class="preview-muted" style={{ margin: '0 0 6px' }}>
        Nothing changes on this PC unless you allow it. You can turn it off any time in Settings → Sharing with phones.
      </p>
      <div class="settings-group rounded-card bg-surface-2/60">
        <RequestList requests={open()} onDone={() => void tick()} />
      </div>
    </BoardSheet>
  );
}

/** UNI-237: "Start at login" (tauri-plugin-autostart) — the app starts hidden
 *  in the tray, so phones can reach this PC from login. */
function AutostartRow() {
  const auto = useAutostart();
  return (
    <div class="settings-row">
      <span class="settings-icon"><Power size={18} /></span>
      <div class="min-w-0">
        <p class="text-[15px] text-text">Start at login</p>
        <p class="text-xs text-text-dim mt-0.5">Opens UniPi hidden in the tray when you log in, so phones can reach this PC right away.</p>
      </div>
      <div class="shrink-0">
        <SwitchField label="Start at login" checked={auto.on()} onChange={(on) => void auto.set(on)} testId="autostart-toggle" />
      </div>
    </div>
  );
}
