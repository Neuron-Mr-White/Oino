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
import { latestPermissionEntry, latestPlanEntry, readSessionWorkflowStatus, sessionWorkflowTopic } from '../../lib/status/plan';
import { AnchoredMenu } from '../board/AnchoredMenu';
import { menuKeys } from './menuKeys';
import { toasts } from '../../lib/toasts';
import { SwitchField } from '../Switch';
import { Sheet } from '../phone';

/** Phone (§3.5): the Mode sheet is open for this session key (the chip, or the chat ⋯ sheet's "Mode" row). */
const [modeSheetFor, setModeSheetFor] = createSignal<string | undefined>();
/** Opens the phone Mode sheet of the composer showing `sessionKey`. */
export function openModeSheet(sessionKey: string): void {
  setModeSheetFor(sessionKey);
}

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

/** Per-host default permission mode (`extensionSettings.permission.defaultMode`), kept in sync with Make default. */
const [hostDefaults, setHostDefaults] = createSignal<Record<string, PermissionMode>>({});
const fetchedHosts = new Set<string>();

function ensureHostDefault(session: SessionHandle): void {
  const hostId = session.hostId;
  if (!hostId || fetchedHosts.has(hostId)) return;
  fetchedHosts.add(hostId);
  session
    .cmd<{ values?: Record<string, unknown> }>({ type: '_oino/get_extension_settings', namespace: 'permission' })
    .then((reply) => {
      const vals = reply?.values ?? {};
      const raw = typeof vals.defaultMode === 'string' ? vals.defaultMode : typeof vals.mode === 'string' ? vals.mode : undefined;
      if (isMode(raw)) setHostDefaults((cur) => ({ ...cur, [hostId]: cur[hostId] ?? raw }));
    })
    .catch(() => {
      fetchedHosts.delete(hostId);
    });
}

/** The mode a session runs in now (G-01 order), read without subscribing. */
function modeOf(session: SessionHandle): PermissionMode {
  const fromTopic = readSessionWorkflowStatus(session.state.topics, session.sessionId())?.permissionMode;
  if (isMode(fromTopic)) return fromTopic;
  const raw = latestPermissionEntry(session.state.entries);
  if (isMode(raw)) return raw;
  return hostDefaults()[session.hostId] ?? 'auto';
}

/** "Auto" — the chat ⋯ sheet's Mode value. */
export function permissionModeLabel(session: SessionHandle): string {
  return MODE_LABEL[modeOf(session)];
}

/** Subscribes to the per-session workflow topic (`unipi:workflow:status:<sessionId>`) and reads real state. */
function useWorkflow(session: () => SessionHandle) {
  createEffect(
    on(
      () => [session(), session().sessionId()] as const,
      ([handle, sid]) => {
        const release = handle.acquire();
        handle.subscribeTopics([sessionWorkflowTopic(sid)]);
        ensureHostDefault(handle);
        onCleanup(release);
      },
    ),
  );
  const status = () => readSessionWorkflowStatus(session().state.topics, session().sessionId());
  const entryMode = () => {
    const raw = latestPermissionEntry(session().state.entries);
    return isMode(raw) ? raw : undefined;
  };
  const planEntry = () => latestPlanEntry(session().state.entries);
  /** G-01: this chat's workflow topic → this chat's persisted entry → the PC's configured default → `auto`. */
  const mode = (): PermissionMode => {
    const fromTopic = status()?.permissionMode;
    if (isMode(fromTopic)) return fromTopic;
    const fromEntry = entryMode();
    if (fromEntry) return fromEntry;
    return hostDefaults()[session().hostId] ?? 'auto';
  };
  /** G-03: plan state from the live topic when subscribed, else the session's `unipi:plan-mode` entry. */
  const planOn = (): boolean => {
    const s = status();
    if (s !== undefined) return s.planMode;
    return planEntry()?.active === true;
  };
  return { status, mode, planOn };
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
  /** Phone (§3.5, PH-29): the chip opens the Mode sheet (modes, Plan before acting, Use for new chats too). */
  phone?: boolean;
}) {
  const { mode: reported, planOn } = useWorkflow(() => props.session);
  /** Optimistic pick for THIS session only — cleared immediately on session switch or when confirmed. */
  const [wanted, setWanted] = createSignal<{ key: string; mode: PermissionMode } | undefined>();
  let wantedTimer: ReturnType<typeof setTimeout> | undefined;
  const mode = (): PermissionMode => {
    const w = wanted();
    return w && w.key === props.session.key ? w.mode : reported();
  };
  createEffect(
    on(
      () => props.session.key,
      () => {
        clearTimeout(wantedTimer);
        setWanted(undefined);
        setOpen(false);
      },
    ),
  );
  createEffect(
    on(reported, (m) => {
      const w = wanted();
      if (w && w.key === props.session.key && m === w.mode) {
        clearTimeout(wantedTimer);
        setWanted(undefined);
      }
    }),
  );
  onCleanup(() => clearTimeout(wantedTimer));
  const available = () => offered(props.session, 'permission');
  const [open, setOpen] = createSignal(false);
  let anchor: HTMLButtonElement | undefined;
  const [anchorRect, setAnchorRect] = createSignal<DOMRect | undefined>();

  const openMenu = () => {
    if (props.disabled) return;
    if (props.phone) {
      setModeSheetFor(props.session.key);
      return;
    }
    setAnchorRect(anchor?.getBoundingClientRect());
    setOpen(true);
  };
  const pick = (value: PermissionMode) => {
    setOpen(false);
    // G-01: never skip sending `/permission <mode>` — picking any mode in the menu always applies it.
    const key = props.session.key;
    setWanted({ key, mode: value });
    clearTimeout(wantedTimer);
    wantedTimer = setTimeout(() => setWanted((cur) => (cur?.key === key ? undefined : cur)), 6000);
    props.onCommand(`/permission ${value}`);
  };
  /** G-09: "Make default for new chats" writes global `defaultMode` (`permission` namespace) on any screen. */
  const makeDefault = () => {
    const value = mode();
    const hostId = props.session.hostId;
    setOpen(false);
    props.session
      .cmd({ type: '_oino/set_extension_setting', namespace: 'permission', key: 'defaultMode', value, scope: 'global' })
      .catch(() =>
        props.session.cmd({ type: '_oino/set_setting', key: 'extensionSettings.permission.defaultMode', value, scope: 'global' }),
      )
      .then(async () => {
        await props.session.cmd({ type: '_oino/set_setting', key: 'extensionSettings.permission.mode', value: null, scope: 'global' }).catch(() => undefined);
        if (hostId) setHostDefaults((cur) => ({ ...cur, [hostId]: value }));
        toasts().push(`New chats start in ${MODE_LABEL[value]}`, 'success');
      })
      .catch(() => toasts().push('Could not save the default', 'error'));
  };

  const sheetOpen = () => !!props.phone && modeSheetFor() === props.session.key;
  const closeSheet = () => setModeSheetFor((cur) => (cur === props.session.key ? undefined : cur));
  const isDefault = () => hostDefaults()[props.session.hostId] === mode();
  const clearDefault = () => {
    const hostId = props.session.hostId;
    props.session
      .cmd({ type: '_oino/set_setting', key: 'extensionSettings.permission.defaultMode', value: null, scope: 'global' })
      .then(() => {
        if (hostId) setHostDefaults((cur) => ({ ...cur, [hostId]: 'auto' }));
        toasts().push('New chats start in Auto', 'success');
      })
      .catch(() => toasts().push('Could not save the default', 'error'));
  };
  const plan = () => offered(props.session, 'plan');

  return (
    <Show when={available()}>
      <Show when={props.phone}>
        <Sheet open={sheetOpen()} onClose={closeSheet} label="Mode" title="Mode" testId="mode-sheet">
          <div class="ph-mode-list" role="radiogroup" aria-label="Permission mode">
            <For each={PERMISSION_MODES}>
              {(value) => (
                <button
                  type="button"
                  role="radio"
                  aria-checked={mode() === value}
                  class="ph-mode-row"
                  onClick={() => {
                    if (mode() !== value) pick(value);
                    closeSheet();
                  }}
                  data-testid={`permission-option-${value}`}
                >
                  <span class="ph-mode-radio" aria-hidden="true" />
                  <span class="ph-mode-text">
                    <span class="ph-mode-label">{MODE_LABEL[value]}</span>
                    <span class="ph-mode-desc">{MODE_HINT[value]}</span>
                  </span>
                </button>
              )}
            </For>
          </div>
          <div class="ph-mode-switches">
            <Show when={plan()}>
              <label class="ph-switch-row">
                <span class="ph-mode-text">
                  <span class="ph-mode-label">Plan before acting</span>
                  <span class="ph-mode-desc">The agent writes a plan you approve first</span>
                </span>
                <SwitchField checked={planOn()} label="Plan before acting" onChange={(on) => props.onCommand(`/plan ${on ? 'on' : 'off'}`)} testId="mode-plan-switch" />
              </label>
            </Show>
            <label class="ph-switch-row">
              <span class="ph-mode-text">
                <span class="ph-mode-label">Use for new chats too</span>
                <span class="ph-mode-desc">New chats on this PC start in {MODE_LABEL[mode()]}</span>
              </span>
              <SwitchField checked={isDefault()} label="Use for new chats too" onChange={(on) => (on ? makeDefault() : clearDefault())} testId="mode-default-switch" />
            </label>
          </div>
        </Sheet>
      </Show>
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
      <Show when={!props.phone && open() && anchorRect()}>
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
          <button type="button" role="menuitem" class="perm-menu-row perm-menu-default" onClick={makeDefault} data-testid="permission-make-default">
            <span class="perm-menu-check" />
            <span class="perm-menu-label">Make default for new chats</span>
          </button>
          <p class="perm-menu-scope" data-testid="permission-scope">Applies to this chat only.</p>
          </div>
        </AnchoredMenu>
      </Show>
    </Show>
  );
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
