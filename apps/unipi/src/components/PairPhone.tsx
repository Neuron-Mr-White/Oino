/**
 * "Pair a phone" (UNI-266, desktop app): this PC shows a QR the phone app
 * scans (Add a PC → Pair with QR). States from lib/pairPhone.ts: starting →
 * waiting (QR, 10:00 countdown, the code, Copy link) → paired / expired
 * (Try again) / error (install or start unipi-host, with the exact command).
 * Opened from Settings → This computer, the Add a PC dialog and the palette.
 */
import { createEffect, createMemo, createSignal, Match, onCleanup, Show, Switch } from 'solid-js';
import { alwaysOn, keepAvailableNote, offerKeepAvailable, type AlwaysOn } from '../lib/keepAvailable';
import { Check, CircleAlert, Clock, Copy, Play, RefreshCw, Smartphone } from 'lucide-solid';
import { encode } from 'uqr';
import { BoardSheet } from './board/BoardSheet';
import { can } from '../lib/platform';
import { copyText } from '../lib/board/share';
import { refreshSharing } from '../lib/sharing';
import { refreshThisComputer } from '../lib/thisComputer';
import {
  closePairPhone,
  countdown,
  groupCode,
  markExpired,
  pairPhoneOpen,
  pairPhoneState,
  startCommand,
  startHostThenPair,
  startPairPhone,
  type PairState,
} from '../lib/pairPhone';
import './pairPhone.css';

/** The link as an SVG QR: dark modules on white with a 4-module quiet zone (scanners want light around it). */
export function QrSvg(props: { value: string; size?: number; label?: string }) {
  const qr = createMemo(() => encode(props.value, { ecc: 'M', border: 4 }));
  const path = createMemo(() => {
    const { data } = qr();
    let d = '';
    data.forEach((row: boolean[], y: number) =>
      row.forEach((dark: boolean, x: number) => {
        if (dark) d += `M${x} ${y}h1v1h-1z`;
      }),
    );
    return d;
  });
  return (
    <svg
      class="pair-phone-qr"
      viewBox={`0 0 ${qr().size} ${qr().size}`}
      width={props.size ?? 280}
      height={props.size ?? 280}
      shape-rendering="crispEdges"
      role="img"
      aria-label={props.label ?? 'Pairing QR code'}
      data-testid="pair-phone-qr"
      data-modules={qr().size}
    >
      <rect width="100%" height="100%" fill="#fff" />
      <path d={path()} fill="#000" />
    </svg>
  );
}

function CommandLine(props: { command: string; testId: string }) {
  const [copied, setCopied] = createSignal(false);
  return (
    <div class="pair-phone-cmd" data-testid={props.testId}>
      <code>{props.command}</code>
      <button
        type="button"
        class="ghost-icon-button"
        aria-label="Copy command"
        onClick={() => void copyText(props.command).then((ok) => ok && (setCopied(true), setTimeout(() => setCopied(false), 1500)))}
      >
        <Show when={copied()} fallback={<Copy size={15} />}>
          <Check size={15} />
        </Show>
      </button>
    </div>
  );
}

function Waiting(props: { s: Extract<PairState, { kind: 'waiting' }> }) {
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  createEffect(() => {
    if (now() >= props.s.expiresAt) markExpired();
  });
  const [copied, setCopied] = createSignal(false);
  const copy = () =>
    void copyText(props.s.link).then((ok) => {
      if (!ok) return;
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  return (
    <div class="pair-phone-body" data-testid="pair-phone-waiting">
      <div class="pair-phone-qr-wrap">
        <QrSvg value={props.s.link} label={`QR code to pair a phone with ${props.s.host}`} />
      </div>
      <p class="pair-phone-steps">
        On your phone, open UniPi → <b>Add a PC</b> → <b>Pair with QR</b> and point it at this code.
      </p>
      <div class="pair-phone-meta">
        <span class="pair-phone-timer" data-testid="pair-phone-countdown" aria-live="off">
          <Clock size={14} /> {countdown(props.s.expiresAt, now())}
        </span>
        <span class="pair-phone-code" data-testid="pair-phone-code" title="Pairing code">
          {groupCode(props.s.code)}
        </span>
      </div>
      <div class="dialog-actions pair-phone-actions">
        <button type="button" class="btn btn-secondary" data-testid="pair-phone-copy" onClick={copy}>
          <Show when={copied()} fallback={<><Copy size={16} /> Copy link</>}>
            <Check size={16} /> Copied
          </Show>
        </button>
      </div>
      <p class="pair-phone-foot">
        No camera handy? Copy the link and paste it in the phone app (Add a PC → Paste pairing link). The code works once, for 10 minutes.
      </p>
    </div>
  );
}

function ErrorState(props: { s: Extract<PairState, { kind: 'error' }> }) {
  const [busy, setBusy] = createSignal(false);
  const [startError, setStartError] = createSignal<string>();
  const start = async () => {
    setBusy(true);
    setStartError(undefined);
    const error = await startHostThenPair();
    setBusy(false);
    if (error) return setStartError(error);
    void refreshSharing();
  };
  const title = () =>
    props.s.errorKind === 'not_installed'
      ? 'This UniPi install is missing its agent host'
      : props.s.errorKind === 'not_running'
        ? "The agent host isn't running"
        : props.s.errorKind === 'too_old'
          ? "This PC's agent host is too old"
          : "Couldn't start pairing";
  return (
    <div class="pair-phone-body pair-phone-state" data-testid="pair-phone-error" data-kind={props.s.errorKind}>
      <span class="pair-phone-state-icon" data-tone="error"><CircleAlert size={26} /></span>
      <h3 class="pair-phone-state-title">{title()}</h3>
      <Switch fallback={<p class="pair-phone-state-text">{props.s.message}{props.s.hint ? ` — ${props.s.hint}` : ''}</p>}>
        <Match when={props.s.errorKind === 'not_installed' || props.s.errorKind === 'too_old'}>
          <p class="pair-phone-state-text" data-testid="pair-phone-fix">
            {props.s.errorKind === 'too_old' ? 'Update UniPi (Settings › About), then try again.' : 'Reinstall UniPi — the app ships its agent host — then try again.'}
          </p>
        </Match>
        <Match when={props.s.errorKind === 'not_running'}>
          <p class="pair-phone-state-text">Phones pair through the host running on this PC. Start it here, or run this in a terminal:</p>
          <CommandLine command={startCommand()} testId="pair-phone-start-cmd" />
        </Match>
      </Switch>
      <Show when={startError()}>
        <p class="pair-phone-state-text preview-field-error" role="alert">{startError()}</p>
      </Show>
      <div class="dialog-actions pair-phone-actions">
        <Show when={props.s.errorKind === 'not_running'}>
          <button type="button" class="btn btn-primary" data-testid="pair-phone-start" disabled={busy()} onClick={() => void start()}>
            <Show when={busy()} fallback={<><Play size={16} /> Start the agent host</>}>
              <RefreshCw size={16} class="refresh-spinning" /> Starting…
            </Show>
          </button>
        </Show>
        <button type="button" class={props.s.errorKind === 'not_running' ? 'btn btn-secondary' : 'btn btn-primary'} data-testid="pair-phone-retry" disabled={busy()} onClick={() => void startPairPhone()}>
          <RefreshCw size={16} /> Try again
        </button>
      </div>
    </div>
  );
}

const EXPIRED_TEXT: Record<string, string> = {
  timeout: 'The code ran out after 10 minutes. Make a new one to pair.',
  replaced: 'Another pairing started on this PC (unipi-oino-host pair), so this code stopped working.',
  locked: 'Too many wrong codes were tried, so this one was switched off.',
  cancelled: 'This code was cancelled.',
};

/** The dialog; mounted once in the app shell. Desktop app only. */
export function PairPhoneDialog() {
  if (!can('localSharing')) return null;
  const s = pairPhoneState;
  // UNI-238 (decision 3): the first phone paired — offer "Keep available when
  // UniPi is closed" by default, and say so in the success state. The note
  // reads this after the offer resolves.
  const [available, setAvailable] = createSignal<AlwaysOn | undefined>();
  const refreshAvailable = async () => setAvailable(await alwaysOn());
  createEffect(() => {
    if (s().kind !== 'paired') return;
    void (refreshSharing(), refreshThisComputer(), offerKeepAvailable().then(refreshAvailable));
  });
  return (
    <BoardSheet open={pairPhoneOpen()} onClose={closePairPhone} label="Pair a phone" testId="pair-phone-sheet" class="pair-phone-sheet">
      <div data-state={s().kind} data-testid="pair-phone">
        <Switch>
          <Match when={s().kind === 'starting' || s().kind === 'idle'}>
            <div class="pair-phone-body pair-phone-state" data-testid="pair-phone-starting">
              <div class="pair-phone-qr-ghost" aria-hidden="true">
                <RefreshCw size={22} class="refresh-spinning" />
              </div>
              <p class="pair-phone-state-text">Making a pairing code…</p>
            </div>
          </Match>
          <Match when={s().kind === 'waiting' && (s() as Extract<PairState, { kind: 'waiting' }>)} keyed>
            {(w) => <Waiting s={w} />}
          </Match>
          <Match when={s().kind === 'paired' && (s() as Extract<PairState, { kind: 'paired' }>)} keyed>
            {(p) => (
              <div class="pair-phone-body pair-phone-state" data-testid="pair-phone-paired">
                <span class="pair-phone-state-icon" data-tone="ok"><Check size={28} /></span>
                <h3 class="pair-phone-state-title">Paired with {p.device}</h3>
                <p class="pair-phone-state-text">It shows up in the phone app now{p.host ? ` as ${p.host}` : ''}.</p>
                <Show when={keepAvailableNote(available())}>
                  {(note) => <p class="pair-phone-state-text" data-testid="pair-phone-keep-available">{note()}</p>}
                </Show>
                <div class="dialog-actions pair-phone-actions">
                  <button type="button" class="btn btn-secondary" data-testid="pair-phone-another" onClick={() => void startPairPhone()}>
                    <Smartphone size={16} /> Pair another phone
                  </button>
                  <button type="button" class="btn btn-primary" data-testid="pair-phone-done" onClick={closePairPhone}>
                    Done
                  </button>
                </div>
              </div>
            )}
          </Match>
          <Match when={s().kind === 'expired' && (s() as Extract<PairState, { kind: 'expired' }>)} keyed>
            {(e) => (
              <div class="pair-phone-body pair-phone-state" data-testid="pair-phone-expired">
                <span class="pair-phone-state-icon"><Clock size={26} /></span>
                <h3 class="pair-phone-state-title">The code expired</h3>
                <p class="pair-phone-state-text">{EXPIRED_TEXT[e.reason] ?? EXPIRED_TEXT.timeout}</p>
                <div class="dialog-actions pair-phone-actions">
                  <button type="button" class="btn btn-primary" data-testid="pair-phone-retry" onClick={() => void startPairPhone()}>
                    <RefreshCw size={16} /> New code
                  </button>
                </div>
              </div>
            )}
          </Match>
          <Match when={s().kind === 'error' && (s() as Extract<PairState, { kind: 'error' }>)} keyed>
            {(e) => <ErrorState s={e} />}
          </Match>
        </Switch>
      </div>
    </BoardSheet>
  );
}
