/**
 * `ReservedHost` (WP-U12, ui-protocol §8): every reserved panel id routes to
 * one **native app component** and one **live owner gate** — no node tree, no
 * second surface (DIRECTION §4.2.2).
 *
 * Where each row lives (app.md §3.7/§3.8/§3.10):
 *
 *   board       rail page `/board` (Board)                      page    unipi.kanboard
 *   cron        rail page `/cron` (Cron)                        page    unipi.cron
 *   files       side-pane `files` tab (`/files` on phone)       pane    host `fs`
 *   git         side-pane `git` tab, Git section                pane    host `git`
 *   browser     side-pane `browser` tab (wave 3, B8)            pane    unipi.browser
 *   terminal    bottom-panel `terminal` tab (wave 3, A29)       pane    host `pty`
 *   sessionInfo Session modal, Info tab (A17)                   overlay unipi.info-screen
 *   lhStatus    side-pane `git` tab, LH section (A16)           pane    unipi.long-horizon
 *   turnFiles   the turn's file-summary footer (A27)            footer  unipi.file-summary
 *
 * An owner's `openPanel{panelId: <reserved id>}` makes the entry appear; this
 * module focuses the native view for it (page, pane tab, modal or footer).
 * Disabling the owner deletes the entry and its nav item/tab goes with it, live
 * (§8 rule 2: "No 'disabled' placeholders"). Where the native component is not
 * available — an earlier phase, the web build, another slot — the owner's
 * `fallback` tree renders instead (§8 rule 3), exactly as the TUI does.
 */
import { useLocation, useNavigate } from '@solidjs/router';
import { createEffect, on, onMount, type Component } from 'solid-js';
import { useHostsOptional } from '../../hosts/context';
import { LOCAL_HOST } from '../../lib/host/client';
import { hostClient } from '../../lib/host';
import { desktopShell, layout } from '../../lib/layout';
import { panes, type PaneArea } from '../../lib/panes';
import { paneKind } from '../../lib/panes/kinds';
import { sessions } from '../../lib/session/store';
import { listExtensions } from '../../settings/hostCmd';
import { sessionUi } from '../registries';
import { FilesTab } from '../../components/files/FilesTab';
import { TurnFilesFooter } from '../../components/timeline/footers/TurnFilesFooter';
import { openSessionModal } from '../../components/session/SessionModal';
import { openStatusSection } from '../../components/status/StatusPane';
import type { UiEntry } from '../types';
import { registerReservedBody, type ReservedBodyProps, type ReservedRow } from './reserved';
import {
  RESERVED_ROWS,
  reservedGatesStale,
  reservedGate,
  reservedRowFor,
  resetReservedGates,
  setHostTools,
  setOwnerGates,
} from './reserved';

// The platform half of the gate lives in `reserved.ts` (§10 capabilities); a
// pane row is only routed when the app actually registered its pane view — the
// browser (B8) and terminal (A29) tabs stay on their owner's fallback tree
// until their `setPaneComponent` lands.
const paneReady = (kind: string | undefined): boolean => !!kind && !!paneKind(kind)?.component;

// ── the native bodies of the rows drawn in a slot (the phone layer) ──────

/**
 * `host/files` drawn in a slot (the phone's full-screen pane layer, §3.9): the
 * real Files tree + editor, rooted at the host the entry belongs to.
 */
const FilesBody: Component<ReservedBodyProps> = (props) => (
  <FilesTab
    tab={{ id: `reserved:${props.entry.id}`, kind: 'files', params: { hostId: props.target.hostId } }}
    area="side"
    hostId={props.target.hostId}
    projectKey="~"
    close={() => undefined}
    setTitle={() => undefined}
  />
);
registerReservedBody('files', FilesBody);

/**
 * `unipi.file-summary/turn-files` drawn where it belongs: inside the turn's row
 * (WP-A27). It reads the turn id off the target (`turnEntryId`, the row's
 * `turnId`) and the data from the extension's `turn` / `diff` / `undo`
 * services (§3.13); with the owner disabled the gate turns off and the owner's
 * fallback tree renders instead (§8 rule 3).
 */
registerReservedBody('turnFiles', TurnFilesFooter);

// ── owner gates (§8 rule 2) ──────────────────────────────────────────────

/** `host_info.tools.unipi_kanboard` when the host answers the v2 shape. */
function kanboardOf(info: unknown): boolean | undefined {
  const tools = (info as { tools?: unknown } | undefined)?.tools;
  if (Array.isArray(tools)) return tools.includes('unipi-kanboard') ? true : undefined;
  if (!tools || typeof tools !== 'object') return undefined;
  const present = (tools as Record<string, unknown>).unipi_kanboard;
  return typeof present === 'boolean' ? present : undefined;
}

/**
 * Refetches the owner state of a host (`_oino/extensions`) and its tools
 * (`host_info`) — the app-side half of the §8 gate: an owner that is off has no
 * reserved surface, live. Called on install and after a Settings toggle
 * (`markReservedGatesStale`).
 */
export async function refreshReservedGates(hostId: string = LOCAL_HOST): Promise<void> {
  const [extensions, info] = await Promise.all([
    listExtensions(hostId).catch(() => undefined),
    Promise.resolve()
      .then(() => hostClient(hostId).request<unknown>('host_info'))
      .catch(() => undefined),
  ]);
  if (extensions?.ok) setOwnerGates(hostId, extensions.value.map((e) => ({ id: e.id, enabled: e.enabled })));
  const kanboard = kanboardOf(info);
  if (kanboard !== undefined) setHostTools(hostId, { kanboard });
}

// ── focusing the native view of an entry (openPanel / palette) ───────────

export type ReservedFocusContext = {
  hostId: string;
  sessionId?: string;
  cwd?: string;
  navigate: (href: string) => void;
  /**
   * Phone: draws the entry's native body in a full-screen layer over the
   * conversation (U10's `PhonePaneLayer`, §13.6 row 6) instead of routing away.
   */
  openLayer?: (entry: UiEntry) => void;
};

/**
 * Entry instances (`id:seq`) whose native view was already focused, so a
 * registry update never re-opens a tab or a page the user closed (§6.4).
 */
const focused = new Set<string>();

/** Pane area of a row's native tab (its default slot, §8). */
const areaOf = (row: ReservedRow): PaneArea => (row.slot === 'bottomPane' ? 'bottom' : 'side');

/** Opens/activates the pane tab (or phone layer) of a pane row; `first` = shown now. */
function focusReservedPane(entry: UiEntry, row: ReservedRow, ctx: ReservedFocusContext): boolean {
  if (!paneReady(row.pane)) return false;
  const path = ctx.cwd || '~';
  // Per project: the entry can arrive before the session's cwd resolves (`~`), and
  // the tab belongs to the session's project scope (§3.8).
  const token = `${entry.id}:${entry.seq}:${path}`;
  if (focused.has(token)) return false;
  focused.add(token);
  const isGit = row.component === 'git' || row.component === 'lhStatus';
  const section = row.component === 'lhStatus' ? ('lh' as const) : undefined;
  if (!desktopShell()) {
    // §3.9/§13.6: a side-pane tab is a full-screen layer on the phone — drawn over
    // the conversation (the native body), never a route change that unmounts it.
    if (row.component === 'files') {
      if (ctx.openLayer) ctx.openLayer(entry);
      else ctx.navigate('/files');
      return true;
    }
    if (isGit) {
      openStatusSection({ hostId: ctx.hostId, sessionId: ctx.sessionId, path }, section);
      return true;
    }
    return false; // browser/terminal layers arrive with their WPs (B8, A29)
  }
  if (isGit) {
    // Z-100: the one Git UI — the reserved entry focuses it at its section.
    openStatusSection({ hostId: ctx.hostId, sessionId: ctx.sessionId, path }, section);
    return true;
  }
  const kind = row.pane as string;
  const area = areaOf(row);
  panes().open({ hostId: ctx.hostId, projectKey: path }, area, { sessionId: ctx.sessionId }, { kind, params: { hostId: ctx.hostId, root: path } });
  if (area === 'side') layout().toggleSide(true);
  else layout().toggleBottom(true);
  return true;
}

/**
 * Focuses the native view of a reserved entry: its page, its pane tab (or the
 * phone's layer), the Session modal, or the footer row it already renders in.
 * `false` when the entry is not reserved or the app cannot render it (the
 * caller then keeps the entry's `fallback` tree).
 */
export function focusReservedEntry(entry: UiEntry, ctx: ReservedFocusContext): boolean {
  const row = reservedRowFor(entry);
  if (!row || !reservedGate(row)) return false;
  switch (row.container) {
    case 'page':
      if (!row.route) return false;
      ctx.navigate(row.route);
      focused.add(`${entry.id}:${entry.seq}`);
      return true;
    case 'pane':
      return focusReservedPane(entry, row, ctx);
    case 'overlay':
      if (row.component !== 'sessionInfo') return false;
      if (!ctx.sessionId) return false; // the modal is session-scoped (A17)
      openSessionModal({ hostId: ctx.hostId, sessionId: ctx.sessionId, tab: 'info', entryId: entry.id });
      focused.add(`${entry.id}:${entry.seq}`);
      return true;
    default:
      return true; // rendered in place (footer/inline) — nothing to focus
  }
}

/** Is this a reserved entry whose native surface is a side/bottom pane tab? */
export function isReservedPaneEntry(entry: UiEntry): boolean {
  const row = reservedRowFor(entry);
  return !!row && row.container === 'pane' && paneReady(row.pane) && reservedGate(row);
}

/** Hash navigation for callers without a router context (the pane reconciler). */
export const hashNavigate = (href: string): void => {
  window.location.hash = `#${href}`;
};

// ── shell install: gates, page focus and the disabled-owner rule ─────────

export type ReservedRoutingOptions = {
  navigate: (href: string) => void;
  pathname: () => string;
  /** The paired hosts the app knows (the shell's host list); This computer is added. */
  hosts?: () => string[];
};

/**
 * Installs the shell half of the routing: fetch the owner gates, open a page
 * row that was opened with `focus` (`/kanboard open` → Board), and close a page
 * whose owner was disabled while it was shown ("focus falls back to Chat",
 * §8 rule 2).
 */
export function installReservedRouting(opts: ReservedRoutingOptions): void {
  // The owners live on the hosts: This computer, the host of the shown session (a
  // phone's PC is rarely "This computer") and every session host the app holds.
  const hostKey = () => {
    const ids = new Set<string>([LOCAL_HOST, ...(opts.hosts?.() ?? [])]);
    const route = /^\/s\/([^/]+)\//.exec(opts.pathname());
    if (route?.[1]) ids.add(decodeURIComponent(route[1]));
    for (const key of sessions().keys()) {
      const [hostId] = key.split('/');
      if (hostId) ids.add(hostId);
    }
    return [...ids].sort().join('|');
  };
  const refresh = () => {
    for (const hostId of hostKey().split('|').filter(Boolean)) void refreshReservedGates(hostId);
  };
  onMount(refresh);
  // A Settings toggle reported stale (or the shown PC changed): refetch, so the
  // nav item/tab follows live.
  createEffect(on(() => `${hostKey()}#${reservedGatesStale()}`, refresh, { defer: true }));
  // §8 rule 2: the open page of an owner that went away closes.
  createEffect(() => {
    const path = opts.pathname();
    const row = RESERVED_ROWS.find((r) => r.container === 'page' && r.route === path);
    if (row && !reservedGate(row)) opts.navigate(desktopShell() ? '/' : '/chat');
  });
  // `openPanel` on a page row with `focus` shows it once per entry instance.
  createEffect(() => {
    for (const key of sessions().keys()) {
      const [hostId, sessionId] = key.split('/');
      if (!hostId || !sessionId) continue;
      const registry = sessionUi(hostId, sessionId);
      const entries = [...registry.stack('sidePane')(), ...registry.entriesFor('fullscreen')()];
      for (const entry of entries) {
        const row = reservedRowFor(entry);
        if (!row || row.container !== 'page' || !entry.focus || !reservedGate(row)) continue;
        const token = `${entry.id}:${entry.seq}`;
        if (focused.has(token)) continue;
        focused.add(token);
        if (row.route && opts.pathname() !== row.route) opts.navigate(row.route);
      }
    }
  });
}

/**
 * Mounted by the shell inside the providers: routes the reserved ids and reads
 * the owner list of every paired host (§8 `ReservedHost`).
 */
export function ReservedRoutingLayer(): null {
  const location = useLocation();
  const navigate = useNavigate();
  const hosts = useHostsOptional();
  installReservedRouting({
    navigate: (href) => navigate(href),
    pathname: () => location.pathname,
    hosts: () => (hosts?.hosts() ?? []).map((host) => host.id),
  });
  return null;
}

/** Tests: forget the owner gates and the per-entry focus guards. */
export function resetReservedHost(): void {
  resetReservedGates();
  focused.clear();
}

export { reservedGate };
