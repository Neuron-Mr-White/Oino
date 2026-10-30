/**
 * UNI-269: the chat's working-mode chip (header, next to the title) and its
 * small popover. The chip shows the long-horizon mode the session runs in —
 * icon + name, tinted per mode, with a live count (`3/7`, `#4`, `45%`) and a
 * pulse while the run goes — and is hidden in plain mode. Since WP-A16
 * (§3.10) the popover only switches the mode (`set_lh_mode`, docs/m5/
 * PROTOCOL.md §7.1) — no live-state body: the LH section of the status pane
 * owns that. In plain mode the same popover opens from ⋯ → Working mode.
 */
import { createMemo, createSignal, For, Show, type JSX } from 'solid-js';
import { Boxes, Check, CircleOff, LoaderCircle, Network, Repeat, Target, Workflow } from 'lucide-solid';
import { AnchoredMenu, MenuSeparator } from '../board/AnchoredMenu';
import type { Chat } from '../../lib/chat/store';
import {
  LH_MODES,
  MODE_DESCRIPTION,
  modeChipCount,
  modeErrorText,
  modeLabel,
  modeView,
  PHASE_WORD,
  switchHint,
  type LhModeChoice,
  type ModeView,
} from '../status/lhMode';

/** Mode icon (unknown future ids get the generic one). */
export function ModeIcon(props: { mode: string; size?: number }): JSX.Element {
  const size = () => props.size ?? 14;
  switch (props.mode) {
    case 'goal':
      return <Target size={size()} />;
    case 'ralph':
      return <Repeat size={size()} />;
    case 'swarm':
      return <Boxes size={size()} />;
    case 'graph':
      return <Workflow size={size()} />;
    case 'none':
      return <CircleOff size={size()} />;
    default:
      return <Network size={size()} />;
  }
}

/** The chat's current mode view (lhMode, falling back to the progress snapshot for older bridges). */
export function chatModeView(chat: Chat | undefined): ModeView | undefined {
  return chat ? modeView(chat.state.lhMode, chat.state.lhProgress) : undefined;
}

export function ModeChip(props: { chat: Chat; onOpen: (anchor: DOMRect) => void }) {
  const view = createMemo(() => chatModeView(props.chat));
  return (
    <Show when={view()}>
      {(v) => (
        <button
          type="button"
          class="mode-chip"
          data-mode={v().mode}
          data-phase={v().phase}
          data-testid="mode-chip"
          aria-label={`Working mode: ${v().label}${v().phase === 'paused' ? ', paused' : ''}. Details`}
          aria-haspopup="menu"
          onClick={(e) => props.onOpen((e.currentTarget as HTMLElement).getBoundingClientRect())}
        >
          <span class="mode-chip-icon">
            <ModeIcon mode={v().mode} size={13} />
          </span>
          <span class="mode-chip-label">{v().label}</span>
          <Show when={v().phase === 'paused'}>
            <span class="mode-chip-count">paused</span>
          </Show>
          <Show when={v().phase !== 'paused' && modeChipCount(v())}>
            {(count) => <span class="mode-chip-count">{count()}</span>}
          </Show>
          <Show when={v().live}>
            <span class="mode-chip-pulse" aria-hidden="true" />
          </Show>
        </button>
      )}
    </Show>
  );
}

export function ModePopover(props: { anchor: DOMRect; chat: Chat; onDismiss: () => void }) {
  const view = createMemo(() => chatModeView(props.chat));
  const current = (): LhModeChoice | string => view()?.mode ?? 'none';
  /** The mode a switch is in flight to. */
  const [pending, setPending] = createSignal<LhModeChoice | null>(null);
  /** A stop of running work waits for a second tap. */
  const [armed, setArmed] = createSignal<LhModeChoice | null>(null);
  const [error, setError] = createSignal<string | undefined>();
  const canSwitch = () => props.chat.state.lhMode !== undefined && props.chat.state.conn === 'live';
  const pick = async (to: LhModeChoice) => {
    setError(undefined);
    const v = view();
    const isCurrent = to === current() && v?.phase !== 'paused';
    if (isCurrent || pending()) return;
    if (to === 'none' && switchHint(v, to) && armed() !== to) {
      setArmed(to);
      return;
    }
    setArmed(null);
    setPending(to);
    try {
      await props.chat.setLhMode(to);
      props.onDismiss();
    } catch (e) {
      setError(modeErrorText(e instanceof Error ? e.message : String(e)));
    } finally {
      setPending(null);
    }
  };
  const choices: LhModeChoice[] = [...LH_MODES, 'none'];
  return (
    <AnchoredMenu anchor={props.anchor} alignEnd label="Working mode" width={300} onDismiss={props.onDismiss}>
      <div class="mode-pop" data-testid="mode-popover">
        <div class="mode-pop-head" data-mode={current()}>
          <span class="mode-pop-icon">
            <ModeIcon mode={current()} size={16} />
          </span>
          <span class="min-w-0 flex-1">
            <span class="mode-pop-title">{view() ? `${view()!.label} mode` : 'Plain chat'}</span>
            <span class="mode-pop-phase" data-phase={view()?.phase}>
              {view() ? PHASE_WORD[view()!.phase] : 'No long-horizon mode'}
            </span>
          </span>
        </div>
        <p class="mode-pop-desc">{MODE_DESCRIPTION[(view()?.mode as LhModeChoice) ?? 'none'] ?? ''}</p>
        <Show when={view()?.phase === 'paused'}>
          <p class="mode-pop-desc" data-testid="mode-paused-note">
            Parked. Send <code>/unipi:{view()!.mode === 'ralph' ? 'ralph' : view()!.mode} resume</code> to pick it up again.
          </p>
        </Show>
      </div>
      <MenuSeparator />
      <div class="board-menu-label">Switch mode</div>
      <Show when={props.chat.state.lhMode === undefined}>
        <p class="board-menu-note" data-testid="mode-unsupported">
          Switching modes from the app needs a newer unipi on this PC. Use /unipi:goal, /unipi:graph… in the chat meanwhile.
        </p>
      </Show>
      <For each={choices}>
        {(mode) => {
          const isCurrent = () => mode === current() && view()?.phase !== 'paused';
          const hint = () => (armed() === mode ? undefined : switchHint(view(), mode));
          return (
            <button
              type="button"
              role="menuitemradio"
              aria-checked={isCurrent()}
              class={`recent-popover-row board-menu-row mode-pop-row${armed() === mode ? ' text-wm-red' : ''}`}
              data-mode={mode}
              data-testid={`mode-pick-${mode}`}
              disabled={!canSwitch() || (pending() !== null && pending() !== mode)}
              onClick={() => void pick(mode)}
            >
              <span class="board-menu-icon mode-pop-pick-icon" data-mode={mode}>
                <ModeIcon mode={mode} size={18} />
              </span>
              <span class="min-w-0 flex-1 text-left">
                <span class="block truncate">{armed() === mode ? `Tap again to stop the ${view()?.mode === 'ralph' ? 'loop' : view()?.label.toLowerCase()}` : mode === 'none' ? 'Plain' : modeLabel(mode)}</span>
                <Show when={hint()}>
                  <span class="mode-pop-hint">{hint()}</span>
                </Show>
              </span>
              <span class="board-menu-hint">
                <Show when={pending() === mode} fallback={<Show when={isCurrent()}><Check size={16} /></Show>}>
                  <LoaderCircle size={16} class="mode-pop-spin" />
                </Show>
              </span>
            </button>
          );
        }}
      </For>
      <Show when={error()}>
        <p class="board-menu-note warn" role="alert" data-testid="mode-error">
          {error()}
        </p>
      </Show>
    </AnchoredMenu>
  );
}
