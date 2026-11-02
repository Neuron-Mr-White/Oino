/**
 * The "this is off on the PC" card in Preview (UNI-248), replacing the bare
 * command box:
 * - viewing **this computer** in the desktop app → the switch itself plus its
 *   options (any port / listed ports, window on the PC);
 * - any other PC → "Ask <PC> to turn this on" with the same options, then a
 *   live pending / approved / denied / expired state (polls while pending).
 * The command stays available under "Or run this on the PC".
 * Desktop streaming is deferred (D-15): its kind is not offered here.
 */
import { createEffect, createSignal, on, onCleanup, Show, type JSX } from 'solid-js';
import { ChevronDown, Clock, Hand, PowerOff, RefreshCw, Send, ShieldCheck, Unplug, X } from 'lucide-solid';
import type { AppError } from '@unipi/protocol';
import { SwitchField } from './Switch';
import { LeaveSheet } from './LeaveSheet';
import { desktopShell } from '../lib/layout';
import {
  ago,
  askStateFrom,
  ASK_POLL_MS,
  changeSharing,
  ENABLE_COMMANDS,
  FEATURE_LABEL,
  HOST_FIX_HINT,
  isOldHost,
  isThisComputer,
  parsePorts,
  scopeText,
  sharingTransport,
  untilText,
  type AskState,
  type FeatureKind as AnyFeatureKind,
  type FeatureRequestItem,
  type FeatureScope,
  type SharingChange,
} from '../lib/sharing';
import { ActionSheet, ListRow } from './phone';
import './sharing.css';

type CommandBoxComp = (props: { command: string }) => JSX.Element;
/** The sharing features this card handles (desktop streaming is deferred, D-15). */
type FeatureKind = Exclude<AnyFeatureKind, 'desktop'>;

const TITLE: Record<FeatureKind, string> = {
  forward: 'Web apps are off on this PC',
  browser: 'The remote browser is off on this PC',
  push: 'Sending files to phones is off on this PC',
  cron: 'Scheduled runs are off on this PC',
};

const WHY: Record<FeatureKind, string> = {
  forward: "Open web apps running on the PC (dev servers on its loopback) here. Only the PC's own loopback is ever reachable.",
  browser: 'A separate Chrome on the PC with its own profile — never your everyday browser or its logins.',
  push: 'Lets the PC send files (like an APK) to this device.',
  cron: "See and manage the PC's scheduled, unattended agent runs.",
};

/** Phone (§3.10, PH-38): the PC is named once, in the title. */
const PHONE_TITLE: Record<FeatureKind, (pc: string) => string> = {
  forward: (pc) => `Web apps are off on ${pc}`,
  browser: (pc) => `The remote browser is off on ${pc}`,
  push: (pc) => `Sending files is off on ${pc}`,
  cron: (pc) => `Scheduled runs are off on ${pc}`,
};

/** Options shared by the switch and the ask card. */
function useOptions(feature: FeatureKind) {
  const [anyPort, setAnyPort] = createSignal(true);
  const [ports, setPorts] = createSignal('');
  const [portsError, setPortsError] = createSignal<string | undefined>();
  const [headed, setHeaded] = createSignal(false);
  /** The scope to ask for / the change to make; `undefined` = invalid input (error shown). */
  const scope = (): FeatureScope | undefined => {
    if (feature === 'forward') {
      if (anyPort()) return { any_port: true };
      const parsed = parsePorts(ports());
      if (!parsed.ok) {
        setPortsError(parsed.message);
        return undefined;
      }
      return { ports: parsed.ports };
    }
    if (feature === 'browser') return headed() ? { headed: true } : {};
    return {};
  };
  const change = (): SharingChange | undefined => {
    const s = scope();
    if (!s) return undefined;
    switch (feature) {
      case 'forward':
        return { kind: 'forward_enable', any: !!s.any_port, ports: s.ports };
      case 'browser':
        return { kind: 'browser_enable', headed: headed() };
      case 'push':
        return { kind: 'push_enable' };
      case 'cron':
        return { kind: 'cron_enable' };
    }
  };
  const [portsSheet, setPortsSheet] = createSignal(false);
  const view = (disabled?: boolean) => (
    <div class="share-options">
      <Show when={feature === 'forward' && !desktopShell()}>
        {/* Phone (§3.12): the port scope is a value row opening a picker sheet, not a segmented control. */}
        <div class="share-phone-row">
          <ListRow title="Which ports" trailing={<span class="share-phone-value">{anyPort() ? 'Any port' : 'Only these ports'}</span>} chevron disabled={disabled} onClick={() => setPortsSheet(true)} testId="share-ports-row" />
        </div>
        <ActionSheet
          open={portsSheet()}
          onClose={() => setPortsSheet(false)}
          title="Which ports"
          testId="share-ports-sheet"
          sections={[[
            { id: 'any', label: 'Any port', description: "Every port on the PC's loopback", checked: anyPort(), onSelect: () => setAnyPort(true) },
            { id: 'only', label: 'Only these ports', description: 'Type the ports below', checked: !anyPort(), onSelect: () => setAnyPort(false) },
          ]]}
        />
      </Show>
      <Show when={feature === 'forward' && desktopShell()}>
        <div class="seg share-seg" role="radiogroup" aria-label="Which ports">
          <button type="button" role="radio" class="seg-option" aria-checked={anyPort()} disabled={disabled} onClick={() => setAnyPort(true)}>
            Any port
          </button>
          <button type="button" role="radio" class="seg-option" aria-checked={!anyPort()} disabled={disabled} onClick={() => setAnyPort(false)}>
            Only these ports
          </button>
        </div>
      </Show>
      <Show when={feature === 'forward'}>
        <Show when={!anyPort()}>
          <input
            class="preview-input share-ports"
            inputmode="numeric"
            placeholder="5173, 8080"
            aria-label="Ports"
            value={ports()}
            disabled={disabled}
            aria-invalid={portsError() ? 'true' : undefined}
            onInput={(e) => {
              setPorts(e.currentTarget.value);
              setPortsError(undefined);
            }}
          />
          <Show when={portsError()}>
            <p class="preview-field-error" role="alert">{portsError()}</p>
          </Show>
        </Show>
      </Show>
      <Show when={feature === 'browser'}>
        <label class="desktop-check share-check">
          <input type="checkbox" checked={headed()} disabled={disabled} onChange={(e) => setHeaded(e.currentTarget.checked)} /> Show its window on the PC (otherwise only in the app)
          too
        </label>
      </Show>
    </div>
  );
  return { scope, change, view };
}

function CommandFold(props: { feature: FeatureKind; CommandBox: CommandBoxComp; label?: string }) {
  const [open, setOpen] = createSignal(false);
  return (
    <div class="share-fold">
      <button type="button" class="share-fold-toggle" aria-expanded={open()} onClick={() => setOpen(!open())}>
        <ChevronDown size={15} class={open() ? 'share-fold-open' : ''} /> {props.label ?? (desktopShell() ? 'Or run this on the PC' : 'Run a command on the PC instead')}
      </button>
      <Show when={open()}>
        <props.CommandBox command={ENABLE_COMMANDS[props.feature]} />
        <p class="preview-muted">No restart needed. Turn it off again with <code>{props.feature === 'cron' ? 'unipi-oino-host cron unshare' : `unipi-oino-host ${props.feature} disable`}</code>.</p>
      </Show>
    </div>
  );
}

export function FeatureOff(props: {
  feature: FeatureKind;
  hostId: string;
  hostName: string;
  CommandBox: CommandBoxComp;
  /** The feature may be on now: re-check the panel. */
  onChanged: () => void;
  testId?: string;
}) {
  return (
    <Show
      when={isThisComputer(props.hostId)}
      fallback={<AskCard {...props} />}
    >
      <LocalSwitch {...props} />
    </Show>
  );
}

function LocalSwitch(props: { feature: FeatureKind; hostName: string; CommandBox: CommandBoxComp; onChanged: () => void; testId?: string }) {
  const options = useOptions(props.feature);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<AppError | undefined>();
  async function turnOn() {
    const change = options.change();
    if (!change) return;
    setBusy(true);
    setError(undefined);
    const r = await changeSharing(change);
    setBusy(false);
    if (!r.ok) {
      setError(r.error);
      return;
    }
    props.onChanged();
  }
  return (
    <section class="preview-notice share-card" data-testid={props.testId ?? `share-local-${props.feature}`}>
      <div class="share-switch-row">
        <div class="preview-notice-icon share-icon"><Unplug size={22} /></div>
        <div class="share-switch-text">
          <h2>{FEATURE_LABEL[props.feature]} on this computer</h2>
          <p class="preview-muted">Off · {WHY[props.feature]}</p>
        </div>
        <SwitchField label={`${FEATURE_LABEL[props.feature]} for phones`} checked={busy()} disabled={busy()} onChange={(v) => v && void turnOn()} testId="share-switch" />
      </div>
      {options.view(busy())}
      <Show when={error()}>
        {(e) => (
          <div class="preview-inline-error" role="alert">
            <X size={18} />
            <div>
              <p>{e().message}</p>
              <Show when={e().code === 'E_NOT_FOUND' || e().code === 'E_UNSUPPORTED'}>
                <p class="preview-muted">{HOST_FIX_HINT}</p>
              </Show>
            </div>
          </div>
        )}
      </Show>
      <p class="preview-muted share-foot">
        Applies at once, no restart. Every paired phone can use it; manage it in Settings → Sharing with phones.
      </p>
      <CommandFold feature={props.feature} CommandBox={props.CommandBox} label="Or use the command" />
    </section>
  );
}

function AskCard(props: { feature: FeatureKind; hostId: string; hostName: string; CommandBox: CommandBoxComp; onChanged: () => void; testId?: string }) {
  const options = useOptions(props.feature);
  const [state, setState] = createSignal<AskState>({ kind: 'idle' });
  const [note, setNote] = createSignal('');
  let timer: ReturnType<typeof setTimeout> | undefined;
  let alive = true;
  onCleanup(() => {
    alive = false;
    clearTimeout(timer);
  });

  async function check(first = false) {
    const host = props.hostId;
    const r = await sharingTransport().requests(host);
    if (!alive || host !== props.hostId) return;
    if (!r.ok) {
      if (isOldHost(r.error)) setState({ kind: 'old-host' });
      // On the first look a failure just leaves the ask card; later ones keep the pending state.
      return;
    }
    const mine = r.value.find((x) => x.feature === props.feature);
    // A stale answer (from before) isn't news on first load: only show what is pending or fresh.
    if (first && mine && mine.status !== 'pending' && Date.now() - (mine.decided_at ?? mine.created_at) > 10 * 60_000) return;
    const next = askStateFrom(mine);
    setState(next);
    if (next.kind === 'approved') props.onChanged();
  }

  createEffect(
    on(
      () => props.hostId,
      () => {
        setState({ kind: 'idle' });
        void check(true);
      },
    ),
  );
  // Poll while pending.
  createEffect(() => {
    clearTimeout(timer);
    if (state().kind !== 'pending') return;
    timer = setTimeout(() => void check(), ASK_POLL_MS);
  });

  async function ask() {
    const scope = options.scope();
    if (!scope) return;
    setState({ kind: 'sending' });
    const r = await sharingTransport().ask(props.hostId, props.feature, Object.keys(scope).length ? scope : undefined, note().trim() || undefined);
    if (!alive) return;
    if (!r.ok) {
      setState(isOldHost(r.error) ? { kind: 'old-host' } : { kind: 'error', error: r.error });
      return;
    }
    setState(askStateFrom(r.value));
    if (r.value.status === 'approved') props.onChanged();
  }

  const s = () => state();
  const request = () => ('request' in s() ? (s() as { request: FeatureRequestItem }).request : undefined);
  const askAgain = () => setState({ kind: 'idle' });

  return (
    <section class="preview-notice share-card" data-tone="warn" data-testid={props.testId ?? `share-ask-${props.feature}`} data-state={s().kind}>
      <div class="preview-notice-icon"><Unplug size={22} /></div>
      <h2>{desktopShell() ? TITLE[props.feature] : PHONE_TITLE[props.feature](props.hostName)}</h2>
      <div class="preview-notice-text">
        <Show when={s().kind === 'idle' || s().kind === 'sending' || s().kind === 'error'}>
          <Show
            when={desktopShell()}
            fallback={<p data-testid="share-ask-body">{WHY[props.feature]} Ask to turn it on: whoever is at the PC gets a notification and decides. Nothing changes until they say yes.</p>}
          >
            <p>
              It's off by default. Ask {props.hostName} to turn it on — whoever is at the PC gets a notification and decides.
              Nothing changes until they say yes.
            </p>
            <p class="preview-muted">{WHY[props.feature]}</p>
          </Show>
          {options.view(s().kind === 'sending')}
          <input
            class="preview-input share-note"
            placeholder={desktopShell() ? 'Add a note (optional)' : 'Note for whoever is at the PC (optional)'}
            aria-label="Note for the PC"
            maxLength={200}
            value={note()}
            disabled={s().kind === 'sending'}
            onInput={(e) => setNote(e.currentTarget.value)}
          />
          <Show when={s().kind === 'error' && (s() as { error: AppError }).error}>
            {(e) => <p class="preview-field-error" role="alert">{e().message}{e().hint ? ` — ${e().hint}` : ''}</p>}
          </Show>
          <button type="button" class="preview-btn primary" data-testid="share-ask" disabled={s().kind === 'sending'} onClick={() => void ask()}>
            <Show when={s().kind === 'sending'} fallback={<Send size={16} />}>
              <RefreshCw size={16} class="refresh-spinning" />
            </Show>
            {s().kind === 'sending' ? 'Asking…' : desktopShell() ? 'Ask the PC to turn this on' : 'Ask to turn on'}
          </button>
        </Show>
        <Show when={s().kind === 'pending' && request()}>
          {(r) => (
            <div class="share-status" data-kind="pending" role="status">
              <Clock size={18} />
              <div>
                <p class="share-status-title">Waiting for {props.hostName}…</p>
                <p class="preview-muted">
                  Asked {ago(r().created_at)}{scopeText(r().feature, r().scope) ? ` for ${scopeText(r().feature, r().scope)}` : ''}. This updates by itself
                  when they answer.
                </p>
              </div>
              <button type="button" class="ghost-icon-button" aria-label="Check now" onClick={() => void check()}>
                <RefreshCw size={16} />
              </button>
            </div>
          )}
        </Show>
        <Show when={s().kind === 'approved' && request()}>{(r) => <ApprovedRow {...props} r={r()} onAfter={() => void check(true)} />}</Show>
        <Show when={s().kind === 'denied'}>
          <div class="share-status" data-kind="denied" role="status">
            <Hand size={18} />
            <div>
              <p class="share-status-title">{props.hostName} said no</p>
              <p class="preview-muted">Nothing was changed on the PC.</p>
            </div>
          </div>
          <button type="button" class="preview-btn" onClick={askAgain}>Ask again</button>
        </Show>
        <Show when={s().kind === 'expired'}>
          <div class="share-status" data-kind="expired" role="status">
            <Clock size={18} />
            <div>
              <p class="share-status-title">{request()?.decided_at && request()?.until ? 'The time ran out' : 'Nobody answered'}</p>
              <p class="preview-muted">{request()?.until ? 'It was switched off again on the PC.' : 'The request expired after a day.'}</p>
            </div>
          </div>
          <button type="button" class="preview-btn" onClick={askAgain}>Ask again</button>
        </Show>
        <Show when={s().kind === 'old-host'}>
          <p>{props.hostName}'s unipi-host is too old to ask from here. Run this on the PC (no restart needed):</p>
          <props.CommandBox command={ENABLE_COMMANDS[props.feature]} />
          <p class="preview-muted">Updating unipi-host on the PC adds the "Ask" button.</p>
        </Show>
        <Show when={s().kind !== 'old-host'}>
          <CommandFold feature={props.feature} CommandBox={props.CommandBox} />
        </Show>
      </div>
    </section>
  );
}

/** The approved state, with UNI-253's way out: "Turn off for this phone". */
function ApprovedRow(props: { feature: FeatureKind; hostId: string; hostName: string; r: FeatureRequestItem; onAfter: () => void }) {
  const [leaving, setLeaving] = createSignal(false);
  return (
    <>
      <div class="share-status" data-kind="approved" role="status">
        <ShieldCheck size={18} />
        <div>
          <p class="share-status-title">{props.hostName} said yes</p>
          <p class="preview-muted">
            On for this device {untilText(props.r.until)}. Opening…
          </p>
        </div>
        <button type="button" class="btn btn-secondary btn-sm" data-testid="leave-open" onClick={() => setLeaving(true)}>
          <PowerOff size={14} /> Turn off
        </button>
      </div>
      <LeaveSheet
        open={leaving()}
        onClose={() => {
          setLeaving(false);
          // Re-read, so the panel stops offering what this phone no longer has.
          props.onAfter();
        }}
        hostId={props.hostId}
        feature={props.feature}
        allowed
      />
    </>
  );
}
