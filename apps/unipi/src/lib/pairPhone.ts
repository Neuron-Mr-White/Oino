/**
 * "Pair a phone" from the desktop app (UNI-266). The core runs this PC's own
 * `unipi-host pair --json --ttl 10m` (src-tauri pair_phone.rs) and streams
 * its JSON lines here; the dialog (components/PairPhone.tsx) draws the QR.
 *
 *   starting → waiting {link, code, expiresAt, host} → paired {device}
 *                                                   → expired {reason}
 *            → error {message, hint, kind}            (Retry starts again)
 *
 * Closing the dialog cancels: unipi-host removes the code, so the QR on the
 * screen stops working.
 */
import { Channel } from '@tauri-apps/api/core';
import { appClient } from './host/factory';
import { createRoot, createSignal } from 'solid-js';
import { can, platform } from './platform';

/** Pairing ops go through the app HostClient (lib/host, WP-A1). */
const invoke = <T = unknown,>(op: string, args?: Record<string, unknown>): Promise<T> => appClient().request<T>(op, args);

export const PAIR_TTL_SECONDS = 600;

export type PairEvent =
  | { state: 'waiting'; link: string; code: string; expires_at: number; host: string }
  | { state: 'paired'; device: string }
  | { state: 'expired'; reason?: 'timeout' | 'replaced' | 'locked' | 'cancelled' }
  | { state: 'error'; message: string; hint?: string; code?: string };

/** What went wrong, for the help under the message. */
export type PairErrorKind = 'not_installed' | 'not_running' | 'too_old' | 'other';

export type PairState =
  | { kind: 'idle' }
  | { kind: 'starting' }
  | { kind: 'waiting'; link: string; code: string; expiresAt: number; host: string }
  | { kind: 'paired'; device: string; host?: string }
  | { kind: 'expired'; reason: 'timeout' | 'replaced' | 'locked' | 'cancelled' }
  | { kind: 'error'; message: string; hint?: string; errorKind: PairErrorKind };

/** The core calls (tests and captures swap them). */
export interface PairPhoneTransport {
  start(onEvent: (e: PairEvent) => void): Promise<void>;
  cancel(): Promise<void>;
  /** Starts the installers' unipi-host service (systemd / logon task / LaunchAgent). Throws an AppError. */
  startHost(): Promise<void>;
}

/** UNI-233: the same attempt through the host's own `pair_begin`/`pair_status`
 * over this PC's local socket (no subprocess); `null` = that path isn't
 * available (no socket / an older host) → the CLI path is used instead. */
let localPoll: { code: string; timer: ReturnType<typeof setInterval> } | undefined;
async function startLocal(onEvent: (e: PairEvent) => void): Promise<boolean> {
  let begun: { link: string; code: string; expires_at: number; host_name: string };
  try {
    begun = (await invoke('local_admin', { req: { op: 'pair_begin', ttl_secs: PAIR_TTL_SECONDS } })) as typeof begun;
  } catch (e) {
    const code = (e as { code?: string })?.code;
    if (code === 'E_NOT_FOUND' || code === 'E_SCHEMA' || code === 'E_UNSUPPORTED' || code === 'E_TRANSPORT') return false;
    throw e;
  }
  onEvent({ state: 'waiting', link: begun.link, code: begun.code, expires_at: begun.expires_at, host: begun.host_name });
  const code = begun.code;
  const timer = setInterval(() => {
    void (invoke('local_admin', { req: { op: 'pair_status', code } }) as Promise<{ state: string; device?: string; reason?: 'timeout' | 'replaced' | 'locked' | 'cancelled' }>)
      .then((st) => {
        if (localPoll?.code !== code) return;
        if (st.state === 'paired') onEvent({ state: 'paired', device: st.device ?? 'Phone' });
        else if (st.state === 'expired') onEvent({ state: 'expired', reason: st.reason });
        else return;
        clearInterval(timer);
        localPoll = undefined;
      })
      .catch(() => {});
  }, 800);
  localPoll = { code, timer };
  return true;
}

const tauriTransport: PairPhoneTransport = {
  async start(onEvent) {
    if (localPoll) clearInterval(localPoll.timer);
    localPoll = undefined;
    if (await startLocal(onEvent)) return;
    const channel = new Channel<PairEvent>();
    channel.onmessage = onEvent;
    await invoke('pair_phone_start', { onEvent: channel });
  },
  async cancel() {
    if (localPoll) {
      const { code, timer } = localPoll;
      clearInterval(timer);
      localPoll = undefined;
      await invoke('local_admin', { req: { op: 'pair_status', code, cancel: true } }).catch(() => {});
      return;
    }
    await invoke('pair_phone_cancel');
  },
  async startHost() {
    await invoke('local_host_start');
  },
};
let override: PairPhoneTransport | undefined;
export function setPairPhoneTransport(t: PairPhoneTransport | undefined): void {
  override = t;
}
const transport = () => override ?? ((window as { __UNIPI_PAIR_PHONE__?: PairPhoneTransport }).__UNIPI_PAIR_PHONE__ ?? tauriTransport);

/** The commands shown when unipi-host is missing / stopped, for this OS. */
export function installCommand(os = platform().os): string {
  return os === 'windows' ? 'irm https://unipi.nrn.one/install.ps1 | iex' : 'curl -fsSL https://unipi.nrn.one/install.sh | sh';
}
export function startCommand(os = platform().os): string {
  if (os === 'windows') return 'schtasks /Run /TN "UniPi Host"';
  if (os === 'macos') return 'launchctl kickstart gui/$(id -u)/dev.unipi.host';
  return 'systemctl --user start unipi-host';
}

export function errorKind(message: string, code?: string): PairErrorKind {
  if (code === 'E_NOT_FOUND' || /isn't installed/i.test(message)) return 'not_installed';
  if (code === 'E_UNSUPPORTED' || /too old/i.test(message)) return 'too_old';
  if (/serve first|isn't running/i.test(message)) return 'not_running';
  return 'other';
}

/** Pure: the next state for an event. Late lines after a final state are ignored. */
export function reduce(state: PairState, e: PairEvent): PairState {
  if (state.kind === 'paired' || state.kind === 'expired' || state.kind === 'error' || state.kind === 'idle') return state;
  switch (e.state) {
    case 'waiting':
      return { kind: 'waiting', link: e.link, code: e.code, expiresAt: e.expires_at * 1000, host: e.host };
    case 'paired':
      return { kind: 'paired', device: e.device || 'Phone', host: state.kind === 'waiting' ? state.host : undefined };
    case 'expired':
      return { kind: 'expired', reason: e.reason ?? 'timeout' };
    case 'error':
      return { kind: 'error', message: e.message, hint: e.hint, errorKind: errorKind(e.message, e.code) };
  }
}

/** "9:58" — whole seconds left until `expiresAt` (ms). */
export function countdown(expiresAt: number, now: number): string {
  const left = Math.max(0, Math.ceil((expiresAt - now) / 1000));
  return `${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`;
}

/** "ABCDE FGHJK" — easier to read out. */
export function groupCode(code: string): string {
  return code.length > 6 ? code.replace(/(.{5})(?=.)/g, '$1 ') : code;
}

function makeStore() {
  return createRoot(() => {
    const [open, setOpen] = createSignal(false);
    const [state, setState] = createSignal<PairState>({ kind: 'idle' });
    return { open, setOpen, state, setState };
  });
}
let store = makeStore();
/** Bumped per start: events from an older attempt are dropped. */
let attempt = 0;

export const pairPhoneOpen = () => store.open();
export const pairPhoneState = () => store.state();

export function resetPairPhone(): void {
  store = makeStore();
  attempt = 0;
}

/** Starts (or restarts) an attempt. */
export async function startPairPhone(): Promise<void> {
  const mine = ++attempt;
  store.setState({ kind: 'starting' });
  try {
    await transport().start((e) => {
      if (mine === attempt) store.setState((s) => reduce(s, e));
    });
  } catch (err) {
    if (mine !== attempt) return;
    const e = err as { message?: string; hint?: string; code?: string };
    const message = e?.message ?? String(err);
    store.setState({ kind: 'error', message, hint: e?.hint, errorKind: errorKind(message, e?.code) });
  }
}

/** "Start unipi-host", then a new attempt. Returns the error text, if any. */
export async function startHostThenPair(): Promise<string | undefined> {
  try {
    await transport().startHost();
  } catch (err) {
    const e = err as { message?: string; hint?: string };
    const message = e?.message ?? String(err);
    return e?.hint ? `${message} — ${e.hint}` : message;
  }
  void startPairPhone();
  return undefined;
}

/** The dialog: opens and starts. Desktop app only. */
export function openPairPhone(): void {
  if (!can('localSharing')) return;
  store.setOpen(true);
  void startPairPhone();
}

/** Closes; a running attempt is cancelled (the code stops working). */
export function closePairPhone(): void {
  const s = store.state();
  attempt++;
  store.setOpen(false);
  if (s.kind === 'starting' || s.kind === 'waiting') void transport().cancel().catch(() => undefined);
  store.setState({ kind: 'idle' });
}

/** The countdown reached zero before the host said so. */
export function markExpired(): void {
  const s = store.state();
  if (s.kind === 'waiting') store.setState({ kind: 'expired', reason: 'timeout' });
}
