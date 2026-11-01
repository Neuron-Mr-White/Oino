/**
 * Permission & plan control (WP-A13, G-6, Z-64).
 *
 * The workflow extension owns the permission gate and plan mode: `/permission
 * ask|auto|full|status` + Alt+M and `/plan on|off|view|approve` + Alt+P, with
 * the sticky `unipi:workflow:status` bus topic carrying
 * `{planMode, permissionMode}` (extensions/unipi/workflow). Until now both
 * were reachable only by typing the command (G-6).
 *
 * This is the visible control: a chip showing the live mode (Ask / Auto /
 * Full, driven by the topic) whose menu sends `/permission <mode>`, and a Plan
 * toggle showing the topic's plan mode and sending `/plan on|off`. Both are
 * bound to the extension's own commands — the app holds no permission state
 * of its own.
 *
 * Hidden when the session does not offer the command (the extension is off),
 * so there is never a dead control.
 */
import { createEffect, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { Check, ChevronUp, ListTree, ShieldCheck } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import { parseWorkflowStatus, WORKFLOW_STATUS_TOPIC } from '../../lib/status/plan';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { menuKeys } from './menuKeys';

/** The three modes the extension's `/permission` accepts (`rules::Mode`). */
export const PERMISSION_MODES = ['ask', 'auto', 'full'] as const;
export type PermissionMode = (typeof PERMISSION_MODES)[number];

const MODE_LABEL: Record<PermissionMode, string> = { ask: 'Ask', auto: 'Auto', full: 'Full' };
const MODE_HINT: Record<PermissionMode, string> = {
  ask: 'Every tool call asks first',
  auto: 'Safe calls run, risky ones ask',
  full: 'Everything runs without asking',
};

const isMode = (value: string | undefined): value is PermissionMode => !!value && (PERMISSION_MODES as readonly string[]).includes(value);

/** Subscribes to the workflow topic and reads it (the control owns no state). */
function useWorkflow(session: () => SessionHandle) {
  createEffect(
    on(session, (handle) => {
      const release = handle.acquire();
      handle.subscribeTopics([WORKFLOW_STATUS_TOPIC]);
      onCleanup(release);
    }),
  );
  const status = () => parseWorkflowStatus(session().state.topics[WORKFLOW_STATUS_TOPIC]?.data);
  /** The extension's default mode (`settings::defaults` → `auto`) until the topic says otherwise. */
  const mode = (): PermissionMode => {
    const value = status()?.permissionMode;
    return isMode(value) ? value : 'auto';
  };
  return { status, mode };
}

/** Whether the session offers one of the workflow extension's commands. */
function offered(session: SessionHandle, name: string): boolean {
  return session.state.commands.some((command) => command.name === name);
}

/** The permission-mode chip: label = live mode, menu = Ask / Auto / Full. */
export function PermissionControl(props: {
  session: SessionHandle;
  /** Sends `/permission <mode>` to this session (never touches the draft). */
  onCommand: (command: string) => void;
  disabled?: boolean;
}) {
  const { mode } = useWorkflow(() => props.session);
  const available = () => offered(props.session, 'permission');
  const [open, setOpen] = createSignal(false);
  let anchor: HTMLButtonElement | undefined;
  const [anchorRect, setAnchorRect] = createSignal<DOMRect | undefined>();

  const openMenu = () => {
    if (props.disabled) return;
    setAnchorRect(anchor?.getBoundingClientRect());
    setOpen(true);
  };
  const pick = (value: PermissionMode) => {
    setOpen(false);
    if (value !== mode()) props.onCommand(`/permission ${value}`);
  };

  return (
    <Show when={available()}>
      <button
        type="button"
        class="tb-chip tb-chip-mode"
        data-mode={mode()}
        aria-label={`Permission mode: ${MODE_LABEL[mode()]}`}
        aria-haspopup="menu"
        aria-expanded={open()}
        title={`Permission: ${MODE_LABEL[mode()]} — ${MODE_HINT[mode()]}`}
        disabled={props.disabled}
        ref={anchor}
        onClick={() => (open() ? setOpen(false) : openMenu())}
        data-testid="permission-chip"
      >
        <ShieldCheck size={15} class="tb-ic" />
        <span class="tb-label" data-testid="permission-label">
          {MODE_LABEL[mode()]}
        </span>
        <ChevronUp size={12} class="tb-chev" />
      </button>
      <Show when={open() && anchorRect()}>
        <AnchoredMenu anchor={anchorRect()!} width={276} label="Permission mode" onDismiss={() => setOpen(false)}>
          <div class="menu-keys" ref={(el) => onCleanup(menuKeys(el, { close: () => setOpen(false), trigger: () => anchor }))}>
          <For each={PERMISSION_MODES}>
            {(value) => (
              <button
                type="button"
                role="menuitemradio"
                aria-checked={mode() === value}
                class="perm-menu-row"
                data-mode={value}
                data-label={MODE_LABEL[value]}
                onClick={() => pick(value)}
                data-testid={`permission-option-${value}`}
              >
                <span class="perm-menu-check">{mode() === value ? <Check size={16} /> : null}</span>
                <span class="perm-menu-label">{MODE_LABEL[value]}</span>
                <span class="perm-menu-desc">{MODE_HINT[value]}</span>
              </button>
            )}
          </For>
          </div>
        </AnchoredMenu>
      </Show>
    </Show>
  );
}

/** The Plan-mode toggle: on = the topic says plan mode is on. */
export function PlanToggle(props: {
  session: SessionHandle;
  onCommand: (command: string) => void;
  disabled?: boolean;
}) {
  const { status } = useWorkflow(() => props.session);
  const available = () => offered(props.session, 'plan');
  const on = () => status()?.planMode === true;
  return (
    <Show when={available()}>
      <button
        type="button"
        class="tb-chip tb-chip-plan"
        data-on={on() ? '' : undefined}
        aria-pressed={on()}
        aria-label={`Plan mode ${on() ? 'on' : 'off'}`}
        title={on() ? 'Plan mode: on — the agent plans before acting' : 'Plan mode: off'}
        disabled={props.disabled}
        onClick={() => props.onCommand(`/plan ${on() ? 'off' : 'on'}`)}
        data-testid="plan-toggle"
      >
        <ListTree size={15} class="tb-ic" />
        <span class="tb-label">Plan</span>
      </button>
    </Show>
  );
}
