/**
 * Sessions (Home, DESIGN §3.1): needs-you strip, ONGOING (live
 * background connections) and RECENT (previously opened, not
 * connected) card grids with previews, hosts section opening the
 * session picker sheet. Nothing on this screen kills a real herdr /
 * tmux session: "Close connection" only detaches our attach stream.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { useLocation, useNavigate } from '@solidjs/router';
import { Clock, Globe, PenLine, Plus, RefreshCw, SquareTerminal, Terminal, Trash2, Unplug } from 'lucide-solid';
import { call } from '../lib/ipc';
import {
  MuxTreeResultSchema,
  type MuxTreeResult,
} from '../lib/schemas';
import { useHosts } from '../hosts/context';
import { useSettings } from '../settings/context';
import { useRecents } from '../lib/recentsContext';
import { recentKey, type Recent } from '../lib/recents';
import {
  closeLive,
  isLive,
  liveByRecency,
  liveSnapshot,
  liveStatus,
  type LiveStatus,
} from '../lib/liveSessions';
import { homeMenuItems, type HomeGroup, type HomeMenuAction } from '../lib/homeMenu';
import { setAddHostOpen } from '../components/Pairing';
import { setTermTarget } from '../lib/termTarget';
import { NoHostState } from '../components/EmptyState';
import { PageHeader } from '../components/PageHeader';
import { ErrorBanner } from '../components/ErrorBanner';
import { BoardSheet } from '../components/board/BoardSheet';
import { SessionPicker } from '../components/SessionPicker';
import { HerdrIcon } from '../components/icons/HerdrIcon';
import { OnlineDot, PathChip } from '../components/ConnectionState';
import { ansiToLines } from '../lib/ansi';
import crab from '../assets/unicrab-pixel.png';
import { PreviewDataSchema, type AppError, type AttachTarget, type TreeData } from '@unipi/protocol';
import { menuAnchor } from '../lib/layout';

/** One card on Home: a live connection (Ongoing) or a recent. */
type HomeCard = {
  key: string;
  group: HomeGroup;
  hostId: string;
  target: AttachTarget | null;
  label: string;
  sublabel: string;
};

export default function Sessions() {
  const hosts = useHosts();
  const [settings] = useSettings();
  const recentsStore = useRecents();
  // Drop recents whose host record no longer exists (re-pair made a
  // new app-side host record with a fresh id).
  const liveRecents = () => {
    const known = new Set(hosts.hosts().map((h) => h.id));
    return recentsStore.recents.filter((r) => known.has(r.hostId));
  };
  const knownHost = (hostId: string) => hosts.hosts().some((h) => h.id === hostId);
  const recentFor = (key: string): Recent | undefined =>
    recentsStore.recents.find((r) => recentKey(r.hostId, r.target) === key);
  /** ONGOING: live background connections, most recently used first. */
  const ongoingCards = (): HomeCard[] =>
    liveByRecency()
      .filter((entry) => knownHost(entry.target.hostId))
      .map((entry) => {
        const recent = recentFor(entry.key);
        return {
          key: entry.key,
          group: 'ongoing' as const,
          hostId: entry.target.hostId,
          target: entry.target.target,
          label: recent?.label ?? entry.target.label,
          sublabel: recent?.sublabel || entry.target.sublabel,
        };
      });
  /** RECENT: previously opened, not connected right now. */
  const recentCards = (): HomeCard[] =>
    liveRecents()
      .filter((recent) => !isLive(recentKey(recent.hostId, recent.target)))
      .map((recent) => ({
        key: recentKey(recent.hostId, recent.target),
        group: 'recent' as const,
        hostId: recent.hostId,
        target: recent.target,
        label: recent.label,
        sublabel: recent.sublabel,
      }));
  const location = useLocation();
  /** Home sits under the terminal on /term: pause its polling there. */
  const onHome = () => location.pathname !== '/term';
  const navigate = useNavigate();
  const [trees, setTrees] = createSignal<Record<string, MuxTreeResult['tree']>>({});
  /** Hosts whose last mux_tree failed (offline): their row says so instead of 'checking…' forever (UNI-218 F56). */
  const [unreachable, setUnreachable] = createSignal<Record<string, boolean>>({});
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<AppError>();
  const [pickerHost, setPickerHost] = createSignal<string | undefined>(undefined);
  const [menu, setMenu] = createSignal<{ card: HomeCard; anchor: DOMRect } | undefined>();
  const [renameRecent, setRenameRecent] = createSignal<Recent | undefined>();
  let start = 0;
  let pollTimer: ReturnType<typeof setInterval> | undefined;

  const onlineHosts = () => hosts.hosts();

  async function refresh() {
    setLoading(true);
    for (const host of onlineHosts()) {
      const result = await call(
        'mux_tree',
        { hostId: host.id, relay: relayArg() },
        MuxTreeResultSchema,
      );
      if (result.ok) {
        setTrees((previous) => ({ ...previous, [host.id]: result.value.tree }));
        setUnreachable((previous) => ({ ...previous, [host.id]: false }));
        hosts.setPath(host.id, result.value.path);
        setError(undefined);
      } else {
        setError(result.error);
        hosts.setPath(host.id, 'unknown');
        setUnreachable((previous) => ({ ...previous, [host.id]: true }));
      }
    }
    setLoading(false);
  }

  function relayArg() {
    const relay = settings.connection.relay;
    return relay.mode === 'custom' ? relay.url : 'n0';
  }

  createEffect(() => {
    // Re-run when the host list loads (list_hosts resolves async).
    hosts.hosts().length;
    void refresh();
  });

  createEffect(() => {
    // Poll trees every 10 s while visible; paused when hidden.
    const timer = setInterval(() => {
      if (!document.hidden && onHome() && hosts.hosts().length) void refresh();
    }, 10_000);
    onCleanup(() => clearInterval(timer));
  });

  /** Opens a terminal: resumes a live background connection
   * instantly, otherwise connects (lib/liveSessions). */
  function openTermFor(hostId: string, target: AttachTarget | null, label: string, sublabel: string) {
    setTermTarget({ hostId, target, label, sublabel });
    navigate('/term');
  }

  function blockedPanes(): { hostId: string; target: AttachTarget; label: string }[] {
    const out: { hostId: string; target: AttachTarget; label: string }[] = [];
    for (const host of onlineHosts()) {
      const tree = trees()[host.id];
      if (!tree?.herdr.available) continue;
      for (const session of tree.herdr.sessions ?? []) {
        if (!session.running) continue;
        for (const workspace of session.workspaces) {
          for (const tab of workspace.tabs) {
            for (const pane of tab.panes) {
              if (pane.agent?.status === 'blocked') {
                out.push({
                  hostId: host.id,
                  target: {
                    mux: 'herdr',
                    session: session.name,
                    workspace_id: workspace.id,
                    tab_id: tab.id,
                    pane_id: pane.id,
                  },
                  label: pane.title || pane.cwd || session.name,
                });
              }
            }
          }
        }
      }
    }
    return out;
  }

  function agentChipFor(hostId: string): string | undefined {
    const tree = trees()[hostId];
    if (!tree?.herdr.available) return undefined;
    for (const session of tree.herdr.sessions ?? []) {
      for (const workspace of session.workspaces) {
        if (workspace.focused && workspace.agent_status !== 'unknown') {
          return workspace.agent_status;
        }
      }
    }
    return undefined;
  }

  function closeMenu() {
    setMenu(undefined);
  }

  /** Runs a long-press menu action. None of them touch the host's
   * real sessions: "close" only detaches our live connection (the
   * card moves to RECENT), "remove" only edits the local list. */
  function runMenuAction(card: HomeCard, action: HomeMenuAction) {
    closeMenu();
    switch (action) {
      case 'open':
        openTermFor(card.hostId, card.target, card.label, card.sublabel);
        return;
      case 'rename': {
        const recent = recentFor(card.key);
        if (recent) setRenameRecent({ ...recent });
        return;
      }
      case 'close-connection':
        closeLive(card.key);
        return;
      case 'remove-recent':
        recentsStore.removeRecent(card.hostId, card.target);
        return;
    }
  }

  return (
    <div
      class="page-screen"
      onTouchStart={(event) => {
        start = event.touches[0].clientY;
      }}
      onTouchEnd={(event) => {
        if (event.changedTouches[0].clientY - start > 90) void refresh();
      }}
    >
      <Show
        when={onlineHosts().length}
        fallback={
          <>
            <PageHeader title="Sessions" />
            <NoHostState what="its terminal sessions" onAdd={() => setAddHostOpen(true)} />
          </>
        }
      >
        <PageHeader
          title="Sessions"
          tools={
            <>
              <button class="ghost-icon-button" aria-label="Preview a web app" onClick={() => navigate('/preview')}>
                <Globe size={20} />
              </button>
              <button class="ghost-icon-button" aria-label="Cron" onClick={() => navigate('/cron')}>
                <Clock size={20} />
              </button>
              <button class="ghost-icon-button" aria-label="Refresh" onClick={() => void refresh()}>
                <RefreshCw size={20} class={loading() ? 'refresh-spinning' : ''} />
              </button>
              <button class="ghost-icon-button" aria-label="Add a PC" onClick={() => setAddHostOpen(true)}>
                <Plus size={20} />
              </button>
            </>
          }
        />
        <div class="page-body">

        <Show when={blockedPanes().length > 0}>
          <button
            role="status"
            class="mb-5 flex h-14 w-full items-center gap-3 rounded-card border border-amber/40 bg-amber/10 px-4 text-left"
            onClick={() => {
              const first = blockedPanes()[0];
              openTermFor(first.hostId, first.target, first.label, 'needs you');
            }}
          >
            <img src={crab} alt="" class="h-7 w-7" style={{ 'image-rendering': 'pixelated' }} />
            <span class="text-[15px] font-medium tabular-nums text-wm-yellow">
              {blockedPanes().length} agent{blockedPanes().length === 1 ? '' : 's'} need
              {blockedPanes().length === 1 ? 's' : ''} you
            </span>
          </button>
        </Show>

        <Show when={ongoingCards().length > 0}>
          <section class="mb-7" aria-label="Ongoing sessions">
            <div class="section-row">
              <h2 class="section-label">Ongoing</h2>
              <span class="section-row-aside">{ongoingCards().length}</span>
            </div>
            <div class="sessions-hgrid" classList={{ 'one-row': ongoingCards().length <= 2 }} data-testid="ongoing-scroll">
              <For each={ongoingCards()}>
                {(card) => (
                  <SessionCard
                    card={card}
                    status={liveStatus(card.key)}
                    previewActive={onHome()}
                    menuOpen={menu()?.card.key === card.key}
                    onOpen={() => openTermFor(card.hostId, card.target, card.label, card.sublabel)}
                    onMenu={(anchor) => setMenu({ card, anchor })}
                  />
                )}
              </For>
            </div>
          </section>
        </Show>

        <Show when={recentCards().length > 0}>
          <section class="mb-7" aria-label="Recent sessions">
            <div class="section-row">
              <h2 class="section-label">Recent</h2>
              <Show when={recentCards().length > 2}>
                <button type="button" class="text-xs text-text-muted" onClick={() => navigate('/chat')}>See all</button>
              </Show>
            </div>
            <div class="sessions-hscroll" data-testid="recent-scroll">
              <For each={recentCards()}>
                {(card) => (
                  <SessionCard
                    card={card}
                    previewActive={onHome()}
                    menuOpen={menu()?.card.key === card.key}
                    onOpen={() => openTermFor(card.hostId, card.target, card.label, card.sublabel)}
                    onMenu={(anchor) => setMenu({ card, anchor })}
                  />
                )}
              </For>
            </div>
          </section>
        </Show>

        <Show when={menu()}>
          {(current) => (
            <RecentMenu anchor={current().anchor} onDismiss={closeMenu}>
              <For
                each={homeMenuItems(current().card.group, {
                  canRename: recentFor(current().card.key) !== undefined,
                })}
              >
                {(item) => (
                  <button
                    class={`recent-popover-row${item.tone === 'danger' ? ' text-wm-red' : ''}`}
                    role="menuitem"
                    onClick={() => runMenuAction(current().card, item.action)}
                  >
                    <MenuIcon action={item.action} /> {item.label}
                  </button>
                )}
              </For>
            </RecentMenu>
          )}
        </Show>

        <section>
          <div class="section-row">
            <h2 class="section-label">PCs</h2>
            <span class="section-row-aside">{onlineHosts().length}</span>
          </div>
          <div class="rounded-card bg-surface-1">
            <For each={onlineHosts()}>
              {(host) => (
                <HostRow
                  host={host}
                  tree={trees()[host.id]}
                  unreachable={unreachable()[host.id] && !trees()[host.id]}
                  onOpen={() => setPickerHost(host.id)}
                />
              )}
            </For>
          </div>
        </section>
        </div>

        <SessionPicker
          target={{
            hostId: pickerHost() ?? '',
            hostName:
              onlineHosts().find((h) => h.id === pickerHost())?.name ?? 'Host',
            relay: settings.connection.relay,
          }}
          open={pickerHost() !== undefined}
          followFocus={settings.terminal.herdrFollowFocus}
          onOpenChange={(open) => {
            if (!open) setPickerHost(undefined);
          }}
          onPreview={() => {
            const hostId = pickerHost();
            setPickerHost(undefined);
            navigate(hostId ? `/preview?host=${encodeURIComponent(hostId)}` : '/preview');
          }}
          onSkip={() => {
            const hostId = pickerHost();
            if (hostId) {
              const name = onlineHosts().find((h) => h.id === hostId)?.name ?? 'Host';
              openTermFor(hostId, null, 'Shell', name);
            }
            setPickerHost(undefined);
          }}
        />
      </Show>
      <Show when={renameRecent()}>
        {(recent) => (
          <RenameLabelSheet
            label={recent().label}
            onClose={() => setRenameRecent(undefined)}
            onApply={(label) => {
              if (label) recentsStore.renameLabel(recent().hostId, recent().target, label);
              setRenameRecent(undefined);
            }}
          />
        )}
      </Show>
    </div>
  );
}

function MenuIcon(props: { action: HomeMenuAction }) {
  switch (props.action) {
    case 'open':
      return <SquareTerminal size={18} />;
    case 'rename':
      return <PenLine size={18} />;
    case 'close-connection':
      return <Unplug size={18} />;
    default:
      return <Trash2 size={18} />;
  }
}

const STATUS_TEXT: Record<LiveStatus, string> = {
  connecting: 'Connecting',
  live: 'Live',
  reconnecting: 'Reconnecting',
  ended: 'Ended',
};

/** A Home card (Ongoing or Recent): preview, label, mux sublabel.
 * Long-press (contextmenu) opens the floating menu. */
function SessionCard(props: {
  card: HomeCard;
  status?: LiveStatus;
  previewActive: boolean;
  menuOpen: boolean;
  onOpen: () => void;
  onMenu: (anchor: DOMRect) => void;
}) {
  return (
    <div
      class="cursor-pointer select-none transition-transform active:scale-[0.97]"
      onClick={() => props.onOpen()}
      data-menu-open={props.menuOpen ? '' : undefined}
      data-group={props.card.group}
      onContextMenu={(event) => {
        event.preventDefault();
        props.onMenu(menuAnchor(event, event.currentTarget as HTMLElement));
      }}
    >
      <RecentThumbnail
        hostId={props.card.hostId}
        target={props.card.target}
        active={props.previewActive}
        localText={props.status ? () => liveSnapshot(props.card.key) : undefined}
      />
      <p class="mt-2 flex items-center gap-1.5 text-[16px] font-semibold leading-5">
        <Show when={props.status}>
          {(status) => (
            <span
              class="live-dot"
              data-status={status()}
              role="img"
              aria-label={STATUS_TEXT[status()]}
              title={STATUS_TEXT[status()]}
            />
          )}
        </Show>
        <span class="truncate">{props.card.label}</span>
      </p>
      <div class="mt-1 flex items-center gap-1.5">
        <Show
          when={props.card.target?.mux === 'herdr'}
          fallback={
            <>
              <Terminal size={14} class="shrink-0 text-text-muted" />
              <span class="truncate text-[14px] leading-5 text-text-muted">{props.card.sublabel}</span>
            </>
          }
        >
          <HerdrIcon size={14} />
          <span class="truncate text-[14px] leading-5 text-wm-green">{props.card.sublabel}</span>
        </Show>
      </div>
    </div>
  );
}

/**
 * Long-press menu for a Recent card: a real floating overlay. Portaled
 * to <body>, `position: fixed`, anchored under (or above) the pressed
 * card and clamped to the viewport, with a transparent tap-to-dismiss
 * backdrop — the page layout never moves.
 */
function RecentMenu(props: { anchor: DOMRect; onDismiss: () => void; children: import('solid-js').JSX.Element }) {
  let menu: HTMLDivElement | undefined;
  const MENU_WIDTH = 220;
  const GAP = 8;
  const [pos, setPos] = createSignal<{ left: number; top: number; origin: string }>({
    left: Math.max(GAP, Math.min(props.anchor.left, window.innerWidth - MENU_WIDTH - GAP)),
    top: props.anchor.bottom + GAP,
    origin: 'top left',
  });
  onMount(() => {
    // offsetHeight, not getBoundingClientRect: the popover-in animation is scaling the menu
    // (0.94) right now, which under-measured it and let tall menus run off the bottom edge.
    const height = menu?.offsetHeight ?? 0;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const nav = document.querySelector('.bottom-nav')?.getBoundingClientRect().height ?? 0;
    const floor = vh - nav - GAP;
    const left = Math.max(GAP, Math.min(props.anchor.left, vw - MENU_WIDTH - GAP));
    let top = props.anchor.bottom + GAP;
    let origin = 'top';
    if (top + height > floor) {
      // Not enough room below: open above the card, else pin to the floor.
      const above = props.anchor.top - GAP - height;
      top = above >= GAP ? above : Math.max(GAP, floor - height);
      origin = above >= GAP ? 'bottom' : 'center';
    }
    const side = left + MENU_WIDTH / 2 < props.anchor.left + props.anchor.width / 2 ? 'right' : 'left';
    setPos({ left, top, origin: `${origin} ${side}` });
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') props.onDismiss();
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('resize', props.onDismiss);
    onCleanup(() => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', props.onDismiss);
    });
  });
  return (
    <Portal>
      <div
        class="recent-popover-backdrop"
        data-testid="recent-menu-backdrop"
        onClick={(event) => {
          // Portal events still bubble to the card in Solid's tree.
          event.stopPropagation();
          props.onDismiss();
        }}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
          props.onDismiss();
        }}
        onTouchMove={() => props.onDismiss()}
      />
      <div
        ref={menu}
        role="menu"
        class="recent-popover"
        style={{
          left: `${pos().left}px`,
          top: `${pos().top}px`,
          'transform-origin': pos().origin,
        }}
        onClick={(event) => event.stopPropagation()}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
        }}
      >
        {props.children}
      </div>
    </Portal>
  );
}

function ChevronRightSimple() {
  return <span class="text-text-dim">›</span>;
}

function RecentThumbnail(props: {
  hostId: string;
  target: AttachTarget | null;
  /** Poll only while Home is actually on screen. */
  active?: boolean;
  /** Ongoing shells: the live xterm's visible text (no host call). */
  localText?: () => string | undefined;
}) {
  const hosts = useHosts();
  const [failed, setFailed] = createSignal(false);
  const [text, setText] = createSignal<string | undefined>(undefined);
  let inFlight = false;

  createEffect(() => {
    // Refresh on mount/Home focus and every 5 s while Home is visible
    // (DESIGN.md §3.1).
    const fetchPreview = () => {
      if (document.hidden || props.active === false) return;
      if (!props.target) {
        const local = props.localText?.();
        if (local !== undefined) setText(local);
        return;
      }
      if (inFlight) return;
      inFlight = true;
      void (async () => {
        const result = await call(
          'mux_preview',
          { hostId: props.hostId, target: props.target, lines: 24 },
          PreviewDataSchema,
        );
        inFlight = false;
        if (result.ok) {
          setFailed(false);
          setText(result.value?.text ?? '');
        } else if (!text()) {
          console.warn(
            `[preview] ${result.error.code}: ${result.error.message} (host ${props.hostId})`,
          );
          setFailed(true);
        }
      })();
    };
    fetchPreview();
    const timer = setInterval(fetchPreview, 5000);
    onCleanup(() => clearInterval(timer));
  });

  return (
    <div class="recent-card relative aspect-[4/3] overflow-hidden rounded-2xl bg-surface-2 p-2.5">
      <div class="pointer-events-none absolute inset-x-0 bottom-0 h-6 bg-gradient-to-t from-surface-2 to-transparent" />
      <Show
        when={props.target || text()}
        fallback={
          <span class="grid h-full w-full place-items-center font-mono text-2xl text-text-muted">
            {'>_'}
          </span>
        }
      >
        <Show
          when={text()}
          fallback={
            failed() ? (
              <span class="grid h-full w-full place-items-center text-[12px] text-text-dim">
                No preview
              </span>
            ) : (
              <div class="skeleton h-full w-full" />
            )
          }
        >
          {(value) => (
            <div class="font-mono text-[10px] leading-[1.35] opacity-80">
              <For each={ansiToLines(value(), 24)}>
                {(line) => (
                  <div class="truncate whitespace-pre">
                    <For each={line}>
                      {(run) => <span style={run.style}>{run.text}</span>}
                    </For>
                  </div>
                )}
              </For>
            </div>
          )}
        </Show>
      </Show>
      <span class="absolute right-2 top-2">
        <PathChip dense path={hosts.paths()[props.hostId] ?? 'unknown'} />
      </span>
    </div>
  );
}

function HostRow(props: {
  host: { id: string; name: string; last_path: 'lan' | 'tailnet' | 'direct' | 'relay' | 'local' | 'unknown' };
  tree: TreeData | undefined;
  unreachable?: boolean;
  onOpen: () => void;
}) {
  const hosts = useHosts();
  // Live route once known (it upgrades relay → LAN after hole punching);
  // the stored route until then.
  const livePath = () => {
    const live = hosts.paths()[props.host.id];
    return live && live !== 'unknown' ? live : props.host.last_path;
  };
  const herdrUp = () => props.tree?.herdr.available ?? false;
  const tmuxUp = () => props.tree?.tmux.available ?? false;
  const sublabel = () => {
    if (!props.tree) return props.unreachable ? "Can't reach this PC" : 'checking…';
    const parts = [herdrUp() ? 'herdr' : null, tmuxUp() ? 'tmux' : null].filter(Boolean);
    return parts.length > 0 ? parts.join(' · ') : 'offline';
  };
  return (
    <button
      class="host-row flex w-full items-center gap-3 border-b border-line/60 px-4 py-3 text-left last:border-b-0"
      onClick={props.onOpen}
    >
      <span class="relative">
        <img src={crab} alt="" class="h-6 w-6" style={{ 'image-rendering': 'pixelated' }} />
        <Show when={props.tree || props.unreachable}>
          <span class="absolute -right-1 -top-1 h-2 w-2 rounded-full" classList={{ 'bg-wm-green': !!props.tree, 'bg-wm-red': !props.tree }} />
        </Show>
      </span>
      <span class="min-w-0 flex-1">
        <span class="block truncate text-[16px] font-semibold leading-[21px]">{props.host.name}</span>
        <span class="block truncate text-[13px]" classList={{ 'text-text-muted': !props.unreachable, 'text-wm-red': !!props.unreachable }} data-testid="host-row-sub">{sublabel()}</span>
      </span>
      <PathChip path={livePath()} />
      <span class="text-text-dim">›</span>
    </button>
  );
}

/** Rename a recent's label (UNI-218 F20): a BoardSheet (centred dialog on the desktop). */
function RenameLabelSheet(props: { label: string; onClose: () => void; onApply: (label: string) => void }) {
  const [value, setValue] = createSignal(props.label);
  const apply = () => props.onApply(value().trim());
  return (
    <BoardSheet
      open
      onClose={props.onClose}
      label="Rename label"
      testId="rename-label-sheet"
      footer={
        <div class="dialog-actions">
          <button type="button" class="btn btn-secondary" onClick={props.onClose}>Cancel</button>
          <button type="button" class="btn btn-primary" disabled={!value().trim()} onClick={apply}>Save</button>
        </div>
      }
    >
      <input
        autofocus
        aria-label="Label"
        class="w-full rounded-input bg-surface-2 px-3 py-3 text-[15px] text-text"
        value={value()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onKeyDown={(e) => { if (e.key === 'Enter' && value().trim()) apply(); }}
      />
    </BoardSheet>
  );
}
