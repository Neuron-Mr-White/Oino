/**
 * Session picker sheet (DESIGN §3.1.1): Corvu drawer with peek/half/
 * full snap points, a Tmux|Herdr segmented capsule, Skip pill, herdr
 * workspace list with drill-in, tmux session list with "In use".
 */
import { createSignal, For, Show, createEffect, onCleanup } from 'solid-js';
import Drawer from 'corvu/drawer';
import { ChevronRight, Globe, Lock, SkipForward, Terminal } from 'lucide-solid';
import { useNavigate } from '@solidjs/router';
import { call } from '../lib/ipc';
import z from 'zod';
import { MuxTreeResultSchema } from '../lib/schemas';
import type { AppError, AttachTarget, MuxKind, TreeData } from '@unipi/protocol';
import { ErrorBanner } from './ErrorBanner';
import { HerdrIcon } from './icons/HerdrIcon';
import { useRecents } from '../lib/recentsContext';
import { setTermTarget } from '../lib/termTarget';
import { pushBack } from '../lib/back';

export type RowMenu =
  | { kind: 'herdr'; session: string; workspaceId: string; label: string }
  | { kind: 'tmux'; session: string; name: string };

export type PickerTarget = {
  hostId: string;
  hostName: string;
  relay?: { mode: 'n0' } | { mode: 'custom'; url: string };
};

type Drill =
  | { kind: 'herdr-tabs'; session: string; workspaceId: string; label: string }
  | { kind: 'herdr-panes'; session: string; tabId: string; label: string }
  | { kind: 'tmux-windows'; session: string }
  | { kind: 'tmux-panes'; session: string; windowId: string };

type HerdrWorkspace = import('@unipi/protocol').HerdrWorkspace;
type HerdrTab = import('@unipi/protocol').HerdrTab;
type HerdrPane = import('@unipi/protocol').HerdrPane;
type TmuxSession = import('@unipi/protocol').TmuxSession;
type TmuxWindow = import('@unipi/protocol').TmuxWindow;
type TmuxPane = import('@unipi/protocol').TmuxPane;

function relativeTime(unix: number, now: number): string {
  const seconds = Math.max(0, Math.floor(now / 1000 - unix));
  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

function shortenCwd(cwd: string | undefined): string {
  if (!cwd) return '';
  return cwd.replace(/^\/home\/[^/]+/, '~');
}

export function SessionPicker(props: {
  target: PickerTarget;
  open: boolean;
  followFocus: boolean;
  onOpenChange: (open: boolean) => void;
  onSkip: () => void;
  /** UNI-213: opens this host's web-app Preview (port forwarding). */
  onPreview?: () => void;
  /** Opened from the terminal itself: swap the target in place
   * (replace the /term history entry, so Back still goes Home). */
  replaceNav?: boolean;
}) {
  const navigate = useNavigate();
  const recents = useRecents();
  const [mux, setMux] = createSignal<MuxKind>(
    recents.pickerMuxFor(props.target.hostId) ?? 'herdr',
  );
  const [tree, setTree] = createSignal<TreeData | undefined>(undefined);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<AppError | undefined>(undefined);
  const [drill, setDrill] = createSignal<Drill | undefined>(undefined);
  const [newForm, setNewForm] = createSignal<'herdr' | 'tmux' | undefined>(undefined);
  const [newName, setNewName] = createSignal('');
  const [newCwd, setNewCwd] = createSignal<string | undefined>(undefined);
  const [menu, setMenu] = createSignal<
    | { kind: 'herdr'; hostId: string; session: string; workspaceId: string; label: string }
    | { kind: 'tmux'; hostId: string; session: string; name: string }
    | undefined
  >(undefined);
  const [menuText, setMenuText] = createSignal('');
  const [actionError, setActionError] = createSignal<string | undefined>(undefined);
  // Half-height by default; only a drag up snaps it to full height,
  // and only then does the top need status-bar clearance.
  const [snap, setSnap] = createSignal<number | string>(0);
  const expanded = () => snap() === 1;

  async function refresh() {
    if (!props.open) return;
    setLoading(true);
    const result = await call(
      'mux_tree',
      { hostId: props.target.hostId, relay: relayArg() },
      MuxTreeResultSchema,
    );
    setLoading(false);
    if (result.ok) {
      setTree(result.value.tree);
      setError(undefined);
    } else {
      setError(result.error);
    }
  }

  const onOpenChange = (open: boolean) => {
    props.onOpenChange(open);
  };

  // Refresh whenever the sheet is opened (controlled or via interaction).
  createEffect(() => {
    if (props.open) void refresh();
  });

  // Android Back closes the open sheet instead of leaving the screen.
  createEffect(() => {
    if (!props.open) return;
    onCleanup(pushBack(() => props.onOpenChange(false)));
  });

  /** Applies a mux_action and refreshes the tree. */
  async function runAction(action: Record<string, unknown> & { hostId?: string }): Promise<boolean> {
    const hostId = action.hostId ?? props.target.hostId;
    const result = await call(
      'mux_action',
      { hostId, relay: relayArg(), action },
      z.unknown(),
    );
    if (result.ok) {
      setActionError(undefined);
      await refresh();
      return true;
    }
    setActionError(result.error.message);
    return false;
  }

  // The create footer targets the first running session that is not
  // the protected `default` (the host refuses creates there anyway);
  // falls back to the first running session.
  const herdrSession = () => {
    const running = herdrSessions(tree()).filter((s) => s.running);
    return (running.find((s) => s.name !== 'default') ?? running[0])?.name;
  };
  const recentDirs = () => distinctCwds(tree());

  function relayArg(): string {
    const relay = props.target.relay;
    return relay?.mode === 'custom' ? relay.url : 'n0';
  }

  function pickMux(next: MuxKind) {
    setMux(next);
    setDrill(undefined);
    recents.setPickerMux(props.target.hostId, next);
    void refresh();
  }

  return (
    <>
      <Drawer
        open={props.open}
        onOpenChange={onOpenChange}
        side="bottom"
        snapPoints={[0, 0.5, 1]}
        defaultSnapPoint={0.5}
        // A fling down from full stops at half instead of closing; every
        // release settles (animated) on 0 / half / full.
        allowSkippingSnapPoints={false}
        onActiveSnapPointChange={setSnap}
      >
      <Drawer.Portal>
        <Drawer.Overlay class="sheet-overlay" />
        <Drawer.Content
          class="picker-sheet picker-full flex h-[100dvh] flex-col"
          data-expanded={expanded() ? '' : undefined}
        >
          <div class="picker-safe-top" aria-hidden="true" />
          <div class="picker-handle" aria-hidden="true" />
          <h2 class="picker-dialog-title truncate">{props.target.hostName}</h2>
          <div class="flex items-center justify-between px-5">
            <div
              class="mux-switch"
              role="radiogroup"
              aria-label="Multiplexer"
              data-active={mux()}
            >
              <span class="mux-switch-thumb" aria-hidden="true" />
              <PickerPill
                selected={mux() === 'tmux'}
                label="Tmux"
                onClick={() => pickMux('tmux')}
              >
                <Terminal size={16} />
              </PickerPill>
              <PickerPill
                selected={mux() === 'herdr'}
                label="Herdr"
                onClick={() => pickMux('herdr')}
              >
                <HerdrIcon size={16} />
              </PickerPill>
            </div>
            <div class="flex items-center gap-2">
              <Show when={props.onPreview}>
                <button
                  class="picker-preview-btn flex h-12 items-center gap-2 rounded-pill bg-surface-2 px-4 text-text"
                  aria-label="Preview a web app from this PC"
                  onClick={() => props.onPreview?.()}
                >
                  <Globe size={16} /> <span class="picker-preview-label">Preview</span>
                </button>
              </Show>
              <button
                class="flex h-12 items-center gap-2 rounded-pill bg-surface-2 px-4 text-text"
                onClick={props.onSkip}
              >
                Skip <SkipForward size={16} />
              </button>
            </div>
          </div>
          <Show when={error()}>
            {(error) => (
              <div class="px-5 pt-3">
                <ErrorBanner error={error()} retry={() => void refresh()} />
              </div>
            )}
          </Show>
          <div class="mt-4 flex-1 min-h-0 overflow-y-auto px-5 pb-[max(1rem,env(safe-area-inset-bottom))]">
            <Show when={loading() && !tree()}>
              <PickerSkeleton />
            </Show>
            <Show when={mux() === 'herdr'}>
              <HerdrList
                tree={tree()}
                drill={drill()}
                setDrill={setDrill}
                followFocus={props.followFocus}
                onMenu={(session, workspaceId, label) => {
                  setMenu({ kind: 'herdr', hostId: props.target.hostId, session, workspaceId, label });
                  setMenuText(label);
                }}
                onOpen={(target, label, sublabel) =>
                  openInTerm(props.target, target, label, sublabel, props.onOpenChange, navigate, props.replaceNav)
                }
              />
            </Show>
            <Show when={mux() === 'tmux'}>
              <TmuxList
                tree={tree()}
                drill={drill()}
                setDrill={setDrill}
                now={Date.now()}
                onMenu={(session, name) => {
                  setMenu({ kind: 'tmux', hostId: props.target.hostId, session, name });
                  setMenuText(name);
                }}
                onOpen={(target, label, sublabel) =>
                  openInTerm(props.target, target, label, sublabel, props.onOpenChange, navigate, props.replaceNav)
                }
              />
            </Show>
            <Show
              when={newForm() === undefined && (mux() === 'tmux' || herdrSession() !== undefined)}
              fallback={
                <Show when={newForm() !== undefined}>
                  <form
                    class="mt-3 rounded-[20px] bg-surface-2 p-4"
                    onSubmit={(event) => {
                      event.preventDefault();
                      void (async () => {
                        const name = newName().trim();
                        if (name === '') return;
                        if (newForm() === 'herdr') {
                          const ok = await runAction({
                            action: 'workspace_create',
                            session: herdrSession() ?? '',
                            label: name,
                            ...(newCwd() ? { cwd: newCwd() } : {}),
                          });
                          if (!ok) return;
                        } else {
                          const ok = await runAction({
                            action: 'tmux_session_create',
                            name,
                            ...(newCwd() ? { cwd: newCwd() } : {}),
                          });
                          if (!ok) return;
                        }
                        setNewForm(undefined);
                        setNewName('');
                        setNewCwd(undefined);
                      })();
                    }}
                  >
                    <p class="mb-2 text-[14px] text-text-muted">
                      {newForm() === 'herdr' ? 'New herdr workspace' : 'New tmux session'}
                    </p>
                    <input
                      class="mb-2 w-full rounded-input bg-surface-1 px-3 py-2 text-[15px] text-text"
                      placeholder={newForm() === 'herdr' ? 'Workspace name' : 'Session name'}
                      value={newName()}
                      onInput={(event) => setNewName(event.currentTarget.value)}
                    />
                    <Show when={recentDirs().length > 0}>
                      <div class="mb-2 flex flex-wrap gap-1.5">
                        <For each={recentDirs()}>
                          {(dir) => (
                            <button
                              type="button"
                              class={`rounded-pill px-2.5 py-1 text-[12px] font-mono ${
                                newCwd() === dir
                                  ? 'bg-surface-1 text-text ring-1 ring-inset ring-line'
                                  : 'bg-surface-1/60 text-text-muted'
                              }`}
                              onClick={() => setNewCwd(newCwd() === dir ? undefined : dir)}
                            >
                              {dir}
                            </button>
                          )}
                        </For>
                      </div>
                    </Show>
                    <Show when={actionError()}>
                      <p class="mb-2 text-[13px] text-wm-red">{actionError()}</p>
                    </Show>
                    <div class="flex items-center justify-end gap-3">
                      <button
                        type="button"
                        class="text-[14px] text-text-muted"
                        onClick={() => {
                          setNewForm(undefined);
                          setNewName('');
                        }}
                      >
                        Cancel
                      </button>
                      <button
                        type="submit"
                        class="rounded-pill bg-shell px-4 py-1.5 text-[14px] text-bg"
                        disabled={newName().trim() === ''}
                      >
                        Create
                      </button>
                    </div>
                  </form>
                </Show>
              }
            >
              <button
                type="button"
                class="mt-3 w-full rounded-[20px] bg-surface-2 py-3.5 text-[15px] text-text-muted"
                onClick={() => {
                  setNewForm(mux());
                  setNewName('');
                  setNewCwd(undefined);
                }}
              >
                + New {mux() === 'herdr' ? 'herdr workspace' : 'tmux session'}
              </button>
            </Show>
          </div>
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer>
    <Show when={menu()}>
      {(menu) => (
        <RowMenuSheet
          menu={menu()}
          text={menuText()}
          setText={setMenuText}
          onClose={() => setMenu(undefined)}
          error={actionError()}
          onApply={(text) => {
            void (async () => {
              const entry = menu();
              if (!entry || text.trim() === '') return;
              // Rename only: the picker never closes/kills a real herdr
              // workspace or tmux session (UNI-128 follow-up).
              const ok =
                entry.kind === 'herdr'
                  ? await runAction({
                      hostId: entry.hostId,
                      action: 'workspace_rename',
                      session: entry.session,
                      workspace_id: entry.workspaceId,
                      label: text.trim(),
                    })
                  : await runAction({
                      hostId: entry.hostId,
                      action: 'tmux_session_rename',
                      session: entry.session,
                      name: text.trim(),
                    });
              if (!ok) return;
              setMenu(undefined);
            })();
          }}
        />
      )}
    </Show>
    </>
  );
}

/// Long-press row menu: rename. There is intentionally no close/kill
/// here: closing from the phone only ever detaches our connection
/// (Home → Ongoing → Close connection); the PC session keeps running.
export function RowMenuSheet(props: {
  menu: NonNullable<RowMenu>;
  text: string;
  setText: (value: string) => void;
  onClose: () => void;
  onApply: (text: string) => void;
  error?: string;
}) {
  return (
    <Drawer open onOpenChange={(open: boolean) => { if (!open) props.onClose(); }} side="bottom" snapPoints={[0, 1]}>
      <Drawer.Portal>
        <Drawer.Overlay class="sheet-overlay" />
        <Drawer.Content class="picker-sheet" aria-label="Row actions">
          <div class="mx-auto mb-3 mt-3 h-1 w-9 rounded-pill bg-line" />
          <div class="px-5 pb-6">
            <p class="mb-3 text-[15px] text-text">
              {props.menu.kind === 'herdr'
                ? `Rename workspace ${props.menu.label}`
                : `Rename session ${props.menu.name}`}
            </p>
            <input
              class="mb-3 w-full rounded-input bg-surface-2 px-3 py-2 text-[15px] text-text"
              value={props.text}
              onInput={(event) => props.setText(event.currentTarget.value)}
              aria-label="New name"
            />
            <div class="flex items-center justify-end gap-4">
              <button type="button" class="text-[14px] text-text-muted" onClick={props.onClose}>
                Cancel
              </button>
              <button
                type="button"
                class="rounded-pill bg-shell px-4 py-1.5 text-[14px] text-bg"
                onClick={() => props.onApply(props.text)}
              >
                Rename
              </button>
            </div>
            <Show when={props.error}>
              <p class="mt-3 text-[13px] text-wm-red">{props.error}</p>
            </Show>
          </div>
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer>
  );
}

function openInTerm(
  picker: PickerTarget,
  target: AttachTarget,
  label: string,
  sublabel: string,
  onOpenChange: (open: boolean) => void,
  navigate: ReturnType<typeof useNavigate>,
  replace = false,
) {
  setTermTarget({
    hostId: picker.hostId,
    target,
    label,
    sublabel: sublabel || picker.hostName,
  });
  onOpenChange(false);
  navigate('/term', replace ? { replace: true } : undefined);
}

function PickerPill(props: {
  selected: boolean;
  label: string;
  onClick: () => void;
  children: import('solid-js').JSX.Element;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={props.selected}
      class={`mux-switch-option ${props.selected ? 'text-text' : 'text-text-muted'}`}
      onClick={props.onClick}
    >
      {props.children}
      {props.label}
    </button>
  );
}

function PickerSkeleton() {
  return (
    <div class="rounded-card bg-surface-2">
      <div class="skeleton mx-4 my-3 h-12 rounded-input" />
      <div class="skeleton mx-4 my-3 h-12 rounded-input" />
      <div class="skeleton mx-4 my-3 h-12 rounded-input" />
    </div>
  );
}

function PickerRow(props: {
  title: string;
  subtitle: string;
  active?: boolean;
  pi?: boolean;
  inUse?: boolean;
  danger?: boolean;
  locked?: boolean;
  onClick?: () => void;
  onDrill?: () => void;
  onMenu?: () => void;
}) {
  return (
    <div
      class={`picker-row mux-row w-full text-left ${props.locked ? 'opacity-45' : ''}`}
      // A row without its own action (a tmux window) drills in, like its chevron.
      // Focusable + Enter/Space so the picker works from a keyboard (desktop).
      role={props.locked ? undefined : 'button'}
      tabindex={props.locked ? undefined : 0}
      aria-disabled={props.locked ? 'true' : undefined}
      onClick={props.locked ? undefined : (props.onClick ?? props.onDrill)}
      onKeyDown={(event) => {
        if (props.locked || event.target !== event.currentTarget) return;
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        (props.onClick ?? props.onDrill)?.();
      }}
      onContextMenu={(event) => {
        if (!props.onMenu) return;
        event.preventDefault();
        props.onMenu();
      }}
    >
      <div class="mux-row-inner ml-5 flex items-center gap-2 border-b border-line/40 py-3.5 pr-5">
        <div class="min-w-0 flex-1">
          <div class={`truncate text-[17px] leading-[22px] ${props.danger ? 'text-wm-red' : 'text-text'}`}>
            {props.title}
          </div>
          <div
            class={`mt-[2px] truncate text-[14px] leading-[18px] ${props.danger ? 'text-wm-red' : 'text-text-muted'}`}
          >
            {props.subtitle}
          </div>
        </div>
        <div class="flex shrink-0 items-center gap-1.5">
          <Show when={props.active}>
            <span class="rounded-pill bg-wm-green/15 px-2 py-0.5 text-xs font-medium text-wm-green">
              Active
            </span>
          </Show>
          <Show when={props.pi}>
            <span
              class="grid h-5 w-5 place-items-center rounded-full bg-surface-3 font-mono text-[11px] text-text-muted"
              title="runs pi"
            >
              π
            </span>
          </Show>
          <Show when={props.inUse}>
            <span class="flex items-center gap-1.5 text-xs text-text-muted">
              <span class="h-2 w-2 rounded-full bg-amber" />
              In use
            </span>
          </Show>
          <Show when={props.onDrill}>
            <button
              type="button"
              aria-label="Drill in"
              class="grid h-10 w-10 place-items-center text-text-dim"
              onClick={(event) => {
                event.stopPropagation();
                props.onDrill?.();
              }}
            >
              <ChevronRight size={16} />
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
}

function herdrSessions(tree: TreeData | undefined) {
  return tree?.herdr.available ? tree.herdr.sessions ?? [] : [];
}

/** Distinct pane cwds from the whole tree, for the new-workspace row. */
function distinctCwds(tree: TreeData | undefined): string[] {
  const out: string[] = [];
  for (const session of herdrSessions(tree)) {
    for (const workspace of session.workspaces) {
      for (const tab of workspace.tabs) {
        for (const pane of tab.panes) {
          if (pane.cwd && !out.includes(pane.cwd)) out.push(pane.cwd);
        }
      }
    }
  }
  for (const session of tmuxSessions(tree)) {
    for (const window of session.windows) {
      for (const pane of window.panes) {
        if (pane.cwd && !out.includes(pane.cwd)) out.push(pane.cwd);
      }
    }
  }
  return out.slice(0, 8);
}

function herdrWorkspaces(tree: TreeData | undefined): HerdrWorkspace[] {
  return herdrSessions(tree).flatMap((s) => s.workspaces);
}

function herdrWorkspacesOf(
  tree: TreeData | undefined,
  session: string,
): HerdrWorkspace[] {
  return herdrSessions(tree)
    .find((s) => s.name === session)?.workspaces ?? [];
}

function herdrTabs(
  tree: TreeData | undefined,
  session: string,
  workspaceId: string,
): HerdrTab[] {
  return (
    herdrWorkspacesOf(tree, session).find((w) => w.id === workspaceId)?.tabs ?? []
  );
}

function herdrPanes(
  tree: TreeData | undefined,
  session: string,
  tabId: string,
): HerdrPane[] {
  // tabId, not workspaceId: the panes drill stores the tab the user
  // tapped, so look the tab up across the session's workspaces.
  const workspace = herdrWorkspacesOf(tree, session).find((w) =>
    w.tabs.some((t) => t.id === tabId),
  );
  return (
    workspace?.tabs.find((t) => t.id === tabId)?.panes ?? []
  );
}

function parentWorkspaceId(
  tree: TreeData | undefined,
  session: string,
  tabId: string,
): string {
  return (
    herdrWorkspacesOf(tree, session).find((w) =>
      w.tabs.some((t) => t.id === tabId),
    )?.id ?? ''
  );
}

function tmuxSessions(tree: TreeData | undefined): TmuxSession[] {
  return tree?.tmux.available ? tree.tmux.sessions ?? [] : [];
}

function tmuxWindows(tree: TreeData | undefined, session: string): TmuxWindow[] {
  return tmuxSessions(tree).find((s) => s.name === session)?.windows ?? [];
}

function tmuxPanes(
  tree: TreeData | undefined,
  session: string,
  windowId: string,
): TmuxPane[] {
  return tmuxWindows(tree, session).find((w) => w.id === windowId)?.panes ?? [];
}

/** Herdr list: drill views first, then the workspace list. */
function HerdrList(props: {
  tree: TreeData | undefined;
  drill: Drill | undefined;
  setDrill: (drill: Drill | undefined) => void;
  followFocus: boolean;
  onMenu?: (session: string, workspaceId: string, label: string) => void;
  onOpen: (target: AttachTarget, label: string, sublabel: string) => void;
}) {
  // Read the context here, in the component body: useRecents() inside
  // an event handler has no owner and throws (UNI-128 "Got it" did
  // nothing because of that).
  const recents = useRecents();
  const sessions = () => herdrSessions(props.tree);
  const isProtected = (name: string) => (props.tree?.protected ?? []).includes(name);
  return (
    <>
      <Show
        when={props.drill?.kind === 'herdr-tabs' ? props.drill : undefined}
        keyed
      >
        {(drill) => (
          <>
            <button
              class="mb-2 flex items-center gap-1.5 text-left"
              onClick={() => props.setDrill(undefined)}
            >
              <ChevronRight size={16} class="rotate-180 text-text-muted" />
              <span class="text-[17px] leading-6 text-text">{drill.label}</span>
              <span class="text-sm text-text-muted tabular-nums">
                {herdrTabs(props.tree, drill.session, drill.workspaceId).length} tabs
              </span>
            </button>
            <div class="picker-card rounded-[20px] bg-surface-2 [&>*:last-child>div]:border-b-0">
              <For each={herdrTabs(props.tree, drill.session, drill.workspaceId)}>
                {(tab) => (
                  <PickerRow
                    title={tab.label}
                    subtitle={`${tab.panes.length} pane${tab.panes.length === 1 ? '' : 's'}`}
                    active={tab.focused}
                    onDrill={() =>
                      props.setDrill({
                        kind: 'herdr-panes',
                        session: drill.session,
                        tabId: tab.id,
                        label: tab.label,
                      })
                    }
                  />
                )}
              </For>
            </div>
          </>
        )}
      </Show>
      <Show
        when={props.drill?.kind === 'herdr-panes' ? props.drill : undefined}
        keyed
      >
        {(drill) => (
          <>
            <button
              class="mb-2 flex items-center gap-1.5 text-left"
              onClick={() => props.setDrill(undefined)}
            >
              <ChevronRight size={16} class="rotate-180 text-text-muted" />
              <span class="text-[17px] leading-6 text-text">{drill.label}</span>
              <span class="text-sm text-text-muted tabular-nums">
                {herdrPanes(props.tree, drill.session, drill.tabId).length} panes
              </span>
            </button>
            <div class="picker-card rounded-[20px] bg-surface-2 [&>*:last-child>div]:border-b-0">
              <For each={herdrPanes(props.tree, drill.session, drill.tabId)}>
                {(pane) => (
                  <PickerRow
                    title={pane.title || pane.terminal_title || `Pane ${pane.id}`}
                    subtitle={shortenCwd(pane.cwd)}
                    active={pane.focused}
                    pi={pane.agent?.name === 'pi'}
                    onClick={() =>
                      props.onOpen(
                        {
                          mux: 'herdr',
                          session: drill.session,
                          workspace_id: parentWorkspaceId(
                            props.tree,
                            drill.session,
                            drill.tabId,
                          ),
                          tab_id: drill.tabId,
                          pane_id: pane.id,
                        },
                        drill.label,
                        drill.session,
                      )
                    }
                  />
                )}
              </For>
            </div>
          </>
        )}
      </Show>
      <Show when={!props.drill}>
        <Show when={props.followFocus && !recents.followFocusNoteDismissed()}>
          <div class="mb-3 flex items-center justify-between gap-3 rounded-2xl bg-surface-2/60 p-3 text-[14px] leading-5 text-text-muted">
            <span>Opening a workspace also switches it on your PC</span>
            <button
              type="button"
              class="-my-2 -mr-1 shrink-0 rounded-pill px-3 py-2.5 text-[13px] font-medium text-shell active:bg-surface-3"
              onClick={() => recents.dismissFollowFocusNote()}
            >
              Got it
            </button>
          </div>
        </Show>
        <For each={sessions()}>
          {(session) => (
            <div class="picker-card mb-3 rounded-[20px] bg-surface-2 [&>*:last-child>div]:border-b-0">
              <Show when={session.error}>
                {(error) => (
                  <PickerRow title={session.name} subtitle={error()} danger />
                )}
              </Show>
              <Show when={!session.error}>
                <Show
                  when={session.running && session.workspaces.length > 0}
                  fallback={
                    <PickerRow
                      title={session.name}
                      subtitle={isProtected(session.name) ? 'Protected on host' : session.running ? 'Running · no workspaces yet' : 'Stopped'}
                      danger={false}
                      locked={isProtected(session.name)}
                      onClick={() =>
                        props.onOpen(
                          { mux: 'herdr', session: session.name },
                          session.name,
                          'herdr',
                        )
                      }
                    />
                  }
                >
                  <For each={session.workspaces}>
                    {(workspace) => (
                      <PickerRow
                        title={workspace.label}
                        subtitle={isProtected(session.name) ? 'Protected on host' : `${session.name} · ${workspace.tabs.length} tab${workspace.tabs.length === 1 ? '' : 's'}`}
                        active={workspace.focused}
                        locked={isProtected(session.name)}
                        pi={workspace.tabs.some((tab) =>
                          tab.panes.some((pane) => pane.agent?.name === 'pi'),
                        )}
                        onMenu={() =>
                          props.onMenu?.(session.name, workspace.id, workspace.label)
                        }
                        onDrill={() =>
                          props.setDrill({
                            kind: 'herdr-tabs',
                            session: session.name,
                            workspaceId: workspace.id,
                            label: workspace.label,
                          })
                        }
                        onClick={() => {
                          const follow = props.followFocus;
                          props.onOpen(
                            follow
                              ? {
                                  mux: 'herdr',
                                  session: session.name,
                                  workspace_id: workspace.id,
                                }
                              : { mux: 'herdr', session: session.name },
                            workspace.label,
                            session.name,
                          );
                        }}
                      />
                    )}
                  </For>
                </Show>
              </Show>
            </div>
          )}
        </For>
      </Show>
    </>
  );
}

function tree_sessions(tree: TreeData | undefined) {
  return tree?.herdr.available ? tree.herdr.sessions ?? [] : [];
}

/** Tmux list: drill views first, then the session list. */
function TmuxList(props: {
  tree: TreeData | undefined;
  drill: Drill | undefined;
  setDrill: (drill: Drill | undefined) => void;
  now: number;
  onMenu?: (session: string, name: string) => void;
  onOpen: (target: AttachTarget, label: string, sublabel: string) => void;
}) {
  const sessions = () => tmuxSessions(props.tree);
  const windowsDrill = () =>
    props.drill?.kind === 'tmux-windows' ? props.drill : undefined;
  const panesDrill = () =>
    props.drill?.kind === 'tmux-panes' ? props.drill : undefined;

  return (
    <>
      <Show
        when={windowsDrill()}
        keyed
      >
        {(drill) => (
          <>
            <button
              class="mb-2 flex items-center gap-2 text-sm text-text-muted"
              onClick={() => props.setDrill(undefined)}
            >
              <ChevronRight size={16} class="rotate-180" />
              {drill.session}
            </button>
            <div class="picker-card rounded-[20px] bg-surface-2 [&>*:last-child>div]:border-b-0">
              <For each={tmuxWindows(props.tree, drill.session)}>
                {(window) => (
                  <PickerRow
                    title={window.name}
                    subtitle={`${window.panes.length} pane${window.panes.length === 1 ? '' : 's'}`}
                    active={window.active}
                    onDrill={() =>
                      props.setDrill({
                        kind: 'tmux-panes',
                        session: drill.session,
                        windowId: window.id,
                      })
                    }
                  />
                )}
              </For>
            </div>
          </>
        )}
      </Show>
      <Show
        when={panesDrill()}
        keyed
      >
        {(drill) => (
          <>
            <button
              class="mb-2 flex items-center gap-2 text-sm text-text-muted"
              onClick={() => props.setDrill({ kind: 'tmux-windows', session: drill.session })}
            >
              <ChevronRight size={16} class="rotate-180" />
              {drill.windowId}
            </button>
            <div class="picker-card rounded-[20px] bg-surface-2 [&>*:last-child>div]:border-b-0">
              <For each={tmuxPanes(props.tree, drill.session, drill.windowId)}>
                {(pane) => (
                  <PickerRow
                    title={pane.current_command}
                    subtitle={shortenCwd(pane.cwd)}
                    active={pane.active}
                    pi={pane.current_command === 'pi'}
                    onClick={() =>
                      props.onOpen(
                        {
                          mux: 'tmux',
                          session: drill.session,
                          window_id: drill.windowId,
                          pane_id: pane.id,
                        },
                        drill.session,
                        'tmux',
                      )
                    }
                  />
                )}
              </For>
            </div>
          </>
        )}
      </Show>
      <Show when={!props.drill}>
        <div class="picker-card rounded-[20px] bg-surface-2 [&>*:last-child>div]:border-b-0">
          <For each={sessions()}>
            {(session) => (
              <PickerRow
                title={session.name}
                subtitle={`${session.window_count} ${session.window_count === 1 ? 'window' : 'windows'} · ${relativeTime(session.activity, props.now)}`}
                inUse={session.attached > 0}
                onMenu={() => props.onMenu?.(session.name, session.name)}
                onClick={() =>
                  props.setDrill({ kind: 'tmux-windows', session: session.name })
                }
              />
            )}
          </For>
        </div>
      </Show>
    </>
  );
}
