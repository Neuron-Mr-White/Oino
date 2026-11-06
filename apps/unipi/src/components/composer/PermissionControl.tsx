/** Host-owned autonomy ceiling. Plan remains an independent workflow control. */
import { createEffect, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ListTree } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { latestPlanEntry, readSessionWorkflowStatus, sessionWorkflowTopic } from '../../lib/status/plan';
import { toasts } from '../../lib/toasts';
import { Sheet } from '../phone';
import { autonomyLevel, AUTONOMY_LEVELS, AUTONOMY_HINT, type AutonomyLevel } from '../../lib/session/autonomy';
import './autonomy.css';

const [modeSheetFor, setModeSheetFor] = createSignal<string>();
export function openModeSheet(sessionKey: string): void { setModeSheetFor(sessionKey); }
export function permissionModeLabel(session: SessionHandle): string { return `Autonomy: ${autonomyLevel(session)[0]!.toUpperCase()}${autonomyLevel(session).slice(1)}`; }
/** Subscribes to the per-session workflow topic (`unipi:workflow:status:<sessionId>`) and reads real state. */
function useWorkflow(session: () => SessionHandle) {
  createEffect(
    on(
      () => [session(), session().sessionId()] as const,
      ([handle, sid]) => {
        const release = handle.acquire();
        handle.subscribeTopics([sessionWorkflowTopic(sid)]);

        onCleanup(release);
      },
    ),
  );
  const status = () => readSessionWorkflowStatus(session().state.topics, session().sessionId());
  /** G-03: plan state from the live topic when subscribed, else the session's `unipi:plan-mode` entry. */
  const planOn = (): boolean => {
    const s = status();
    if (s !== undefined) return s.planMode;
    return latestPlanEntry(session().state.entries)?.active === true;
  };
  return { status, planOn };
}

/** Whether the session offers one of the workflow extension's commands. */
function offered(session: SessionHandle, name: string): boolean {
  return session.state.commands.some((command) => command.name === name);
}


export function PermissionControl(props: { session: SessionHandle; onCommand: (command: string) => void; disabled?: boolean; phone?: boolean }) {
  const [pending, setPending] = createSignal(false);
  const level = () => autonomyLevel(props.session);
  const pick = async (value: AutonomyLevel) => {
    if (pending() || props.disabled) return;
    const session = props.session;
    setPending(true);
    try { await session.cmd({ type: 'set_autonomy', level: value }); }
    catch { toasts().push('Could not change autonomy', 'error'); }
    finally { setPending(false); }
  };
  return <>
    <button type="button" class="tb-chip tb-chip-autonomy" data-testid="permission-chip"
      aria-label={AUTONOMY_HINT[level()]} data-tip={AUTONOMY_HINT[level()]} title={AUTONOMY_HINT[level()]}
      disabled={props.disabled || pending()} aria-busy={pending()}
      onClick={() => void pick(AUTONOMY_LEVELS[(AUTONOMY_LEVELS.indexOf(level()) + 1) % 3]!)}>
      <span>Autonomy</span><span class="autonomy-meter" aria-hidden="true">{level() === 'low' ? '▮▯▯' : level() === 'medium' ? '▮▮▯' : '▮▮▮'}</span>
    </button>
    <Show when={props.phone}>
      <Sheet open={modeSheetFor() === props.session.key} onClose={() => setModeSheetFor(undefined)} label="Autonomy" title="Autonomy" testId="mode-sheet">
        <div class="ph-mode-list" role="radiogroup" aria-label="Autonomy">
          <For each={AUTONOMY_LEVELS}>{(value) => <button type="button" class="ph-mode-row" role="radio" aria-checked={level() === value} disabled={pending()}
            onClick={() => { void pick(value); setModeSheetFor(undefined); }}>{AUTONOMY_HINT[value]}</button>}</For>
        </div>
      </Sheet>
    </Show>
  </>;
}

/** The Plan-mode toggle: on = the topic / plan entry says plan mode is on; toggles explicitly `/plan on` / `/plan off`. */
export function PlanToggle(props: {
  session: SessionHandle;
  onCommand: (command: string) => void;
  disabled?: boolean;
}) {
  const { planOn: reported } = useWorkflow(() => props.session);
  const [wanted, setWanted] = createSignal<{ key: string; on: boolean } | undefined>();
  let wantedTimer: ReturnType<typeof setTimeout> | undefined;
  const isOn = () => {
    const w = wanted();
    return w && w.key === props.session.key ? w.on : reported();
  };
  createEffect(
    on(
      () => props.session.key,
      () => {
        clearTimeout(wantedTimer);
        setWanted(undefined);
      },
    ),
  );
  createEffect(
    on(reported, (r) => {
      const w = wanted();
      if (w && w.key === props.session.key && r === w.on) {
        clearTimeout(wantedTimer);
        setWanted(undefined);
      }
    }),
  );
  onCleanup(() => clearTimeout(wantedTimer));
  const available = () => offered(props.session, 'plan');
  const toggle = () => {
    const next = !isOn();
    const key = props.session.key;
    setWanted({ key, on: next });
    clearTimeout(wantedTimer);
    wantedTimer = setTimeout(() => setWanted((cur) => (cur?.key === key ? undefined : cur)), 6000);
    props.onCommand(`/plan ${next ? 'on' : 'off'}`);
  };
  return (
    <Show when={available()}>
      <button
        type="button"
        class="tb-chip tb-chip-plan"
        data-on={isOn() ? '' : undefined}
        aria-pressed={isOn()}
        aria-label={`Plan mode ${isOn() ? 'on' : 'off'}`}
        title={isOn() ? 'Plan mode: on — the agent plans before acting' : 'Plan mode: off'}
        disabled={props.disabled}
        onClick={toggle}
        data-testid="plan-toggle"
      >
        <ListTree size={15} class="tb-ic" />
        <span class="tb-label">Plan</span>
      </button>
    </Show>
  );
}
