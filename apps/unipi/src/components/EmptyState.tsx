/**
 * The one empty / first-run / error state (UNI-218 F10–F12, DESIGN.md §1.5–1.6):
 * crab or icon, title, one short line, actions. `compact` for narrow list
 * columns and inline list states. Anchored to the top of its box at a fixed
 * offset (not vh), so it doesn't drift with the window height.
 */
import { Show, createSignal, type JSX } from 'solid-js';
import { CloudOff, RefreshCw } from 'lucide-solid';
import crab from '../assets/unicrab-pixel.png';
import { isUnreachableCode } from '../lib/host/client';
import { canRestartHost, restartAgentHost } from '../lib/host/restart';
import { desktopShell } from '../lib/layout';
import { Button, Sheet } from './phone';
import './emptyState.css';

export function EmptyState(props: {
  title: string;
  /** One line under the title. A lore line fits screens with nothing to do. */
  body?: JSX.Element;
  /** An icon instead of the crab. */
  icon?: JSX.Element;
  action?: JSX.Element;
  compact?: boolean;
  tone?: 'error';
  testId?: string;
  role?: 'status' | 'alert';
}) {
  return (
    <div
      class="empty-state"
      data-compact={props.compact ? '' : undefined}
      data-tone={props.tone}
      data-testid={props.testId ?? 'empty-state'}
      role={props.role ?? (props.tone === 'error' ? 'alert' : 'status')}
    >
      <Show when={props.icon} fallback={<img src={crab} class="crab" alt="" />}>
        <span class="empty-state-icon">{props.icon}</span>
      </Show>
      <h2>{props.title}</h2>
      <Show when={props.body}>
        <p>{props.body}</p>
      </Show>
      <Show when={props.action}>
        <div class="empty-state-actions">{props.action}</div>
      </Show>
    </div>
  );
}

/** Shared "no PC paired yet" state for every host-dependent screen (F12);
 *  `HostGate` (ConnectionState.tsx) picks it, OfflineState or the content.
 *  Phone (§3.1): the Chats tab gets the first-run onboarding instead. */
export function NoHostState(props: { what: string; onAdd: () => void; onboarding?: boolean }) {
  const onboarding = () => props.onboarding ?? (!desktopShell() && /sessions|chats/i.test(props.what));
  return (
    <Show when={!onboarding()} fallback={<PhoneOnboarding onAdd={props.onAdd} />}>
      <NoHostCard what={props.what} onAdd={props.onAdd} />
    </Show>
  );
}

function NoHostCard(props: { what: string; onAdd: () => void }) {
  return (
    <EmptyState
      title="No PC paired yet"
      body={`Pair Unipi with your PC to see ${props.what} here.`}
      testId="no-host-state"
      action={
        <button type="button" class="empty-add bg-shell text-face rounded-pill" onClick={props.onAdd}>
          Add a PC
        </button>
      }
    />
  );
}

/**
 * First run on the phone (§3.1): crab, "Connect Unipi to your PC", one line,
 * primary "Add a PC" (opens the pairing sheet) and "How it works" (3 steps).
 */
export function PhoneOnboarding(props: { onAdd: () => void }) {
  const [how, setHow] = createSignal(false);
  return (
    <div class="onboard" data-testid="no-host-state" data-onboarding="" role="status">
      <img src={crab} class="onboard-crab" alt="" />
      <h2 class="onboard-title">Connect Unipi to your PC</h2>
      <p class="onboard-body">Your chats run on your PC. This phone shows them and lets you reply from anywhere.</p>
      <div class="onboard-actions">
        <Button kind="primary" block onClick={props.onAdd} testId="onboard-add">Add a PC</Button>
        <Button kind="ghost" block onClick={() => setHow(true)} testId="onboard-how">How it works</Button>
      </div>
      <Sheet open={how()} onClose={() => setHow(false)} label="How it works" title="How it works" testId="onboard-how-sheet"
        footer={<Button kind="primary" block onClick={() => (setHow(false), props.onAdd())}>Add a PC</Button>}>
        <ol class="onboard-steps">
          <li><b>Install Unipi on your PC.</b> It runs your chats, files and agents there.</li>
          <li><b>On the PC, open Settings › Hosts › Pair a phone.</b> It shows a QR code and a link.</li>
          <li><b>Here, tap Add a PC</b> and scan the code — or paste the link.</li>
        </ol>
      </Sheet>
    </div>
  );
}

/**
 * The one "PC unreachable" state (UNI-218 F56): every host screen shows the
 * same error-tone EmptyState — "Can't reach <PC>", the host's hint, Retry —
 * instead of a banner on one screen, a red card on another and a fake empty
 * folder on a third.
 */
export function OfflineState(props: {
  pc?: string;
  message?: string;
  hint?: string;
  onRetry?: () => void;
  compact?: boolean;
  /** A second action after Retry (Board: "Switch PC"). */
  secondary?: { label: string; run: () => void };
  /** This computer (the local agent host): calmer title + "Restart agent host". */
  local?: boolean;
}) {
  const [restarting, setRestarting] = createSignal(false);
  const restart = async () => {
    setRestarting(true);
    await restartAgentHost();
    setRestarting(false);
    props.onRetry?.();
  };
  return (
    <EmptyState
      tone="error"
      compact={props.compact}
      icon={<CloudOff size={props.compact ? 20 : 24} />}
      title={props.local ? "This computer isn't answering" : `Can't reach ${props.pc ?? 'the PC'}`}
      body={props.hint ?? props.message ?? 'Is Unipi running on the PC?'}
      testId="offline-state"
      action={
        props.onRetry || props.secondary || (props.local && canRestartHost()) ? (
          <>
            <Show when={props.onRetry}>
              <button type="button" class="btn btn-secondary" onClick={() => props.onRetry?.()}>
                <RefreshCw size={16} /> Retry
              </button>
            </Show>
            <Show when={props.local && canRestartHost()}>
              <button type="button" class="btn btn-ghost" data-testid="restart-host" disabled={restarting()} onClick={() => void restart()}>
                {restarting() ? 'Restarting…' : 'Restart agent host'}
              </button>
            </Show>
            <Show when={props.secondary}>
              {(secondary) => (
                <button type="button" class="btn btn-ghost" onClick={secondary().run}>
                  {secondary().label}
                </button>
              )}
            </Show>
          </>
        ) : undefined
      }
    />
  );
}

/** Error codes that mean "the PC can't be reached" (show OfflineState, not a generic error).
 *  Same rule the HostClient uses for its connection state (lib/host, Z-90). */
export function isUnreachable(code: string | undefined): boolean {
  return isUnreachableCode(code);
}
