/**
 * Reserved panel ids (ui-protocol §8, WP-U12): the id → native component
 * table, the owner gates and the app's native-surface registry.
 *
 * Reserved entries are **not** node trees: the owner opens one with
 * `openPanel{panelId: <reserved id>}` and the app renders its own component
 * (Board, Cron, Files, Git, Browser, Terminal, Session modal, LH section,
 * turn file summary). Rule 2 of §8 makes the entry's presence the gate: an
 * owner that is disabled (app toggle, safe mode, crash) has no entry, and the
 * app hides the nav item/tab and closes the open view. Rules 1/3 add the
 * host-owned rows (gated on the host service the app talks to) and the
 * fallback: where the native component is not available the owner's
 * `fallback` tree renders instead.
 *
 * This module is a leaf on purpose — the pane registry, the rail, the slot
 * hosts and the shell all read a gate, and a cycle through the screens would
 * break the first import. The components and the routing live in
 * `ReservedHost.tsx`; the owner list comes from the host (`_oino/extensions`),
 * refreshed by `ReservedHost` and by the Settings toggle.
 */
import { createSignal, type Component } from 'solid-js';
import { can, type Capability } from '../../lib/platform';
import type { UiTarget } from '../actions';
import type { Mode } from '../nodes/props';
import type { Slot, UiEntry } from '../types';

/** The nine reserved components of §8 / §10 (`ReservedComponent` on the wire). */
export type ReservedComponent = 'board' | 'cron' | 'files' | 'git' | 'browser' | 'terminal' | 'sessionInfo' | 'lhStatus' | 'turnFiles';

/** Where the app's native surface for a row lives (§3.7 routes, §3.8 panes, §3.10 sections). */
export type ReservedContainer = 'page' | 'pane' | 'overlay' | 'footer' | 'inline';

export type ReservedRow = {
  /** Wire component name (§10). */
  component: ReservedComponent;
  /** Reserved entry id (§8). */
  id: string;
  /** Owning extension id; `host` for the rows the host owns (services, not a manifest). */
  owner: string;
  /** Default slot of the row (§8). */
  slot: Slot;
  /** The app surface that renders it. */
  container: ReservedContainer;
  /** Pane kind of a `pane` row (`lib/panes/kinds.ts`). */
  pane?: string;
  /** Route of the native page (`page` rows; phone layers included). */
  route?: string;
  /** Platform capability that stands for the host service of the row. */
  cap?: Capability;
};

/** The §8 table, in spec order. */
export const RESERVED_ROWS: readonly ReservedRow[] = [
  { component: 'board', id: 'unipi.kanboard/board', owner: 'unipi.kanboard', slot: 'fullscreen', container: 'page', route: '/board', cap: 'board' },
  { component: 'cron', id: 'unipi.cron/jobs', owner: 'unipi.cron', slot: 'fullscreen', container: 'page', route: '/cron', cap: 'chat' },
  { component: 'files', id: 'host/files', owner: 'host', slot: 'sidePane', container: 'pane', pane: 'files', route: '/files', cap: 'files' },
  { component: 'git', id: 'host/git', owner: 'host', slot: 'sidePane', container: 'pane', pane: 'git', cap: 'files' },
  { component: 'browser', id: 'unipi.browser/browser', owner: 'unipi.browser', slot: 'sidePane', container: 'pane', pane: 'browser', cap: 'hosts' },
  { component: 'terminal', id: 'host/terminal', owner: 'host', slot: 'bottomPane', container: 'pane', pane: 'terminal', cap: 'terminal' },
  { component: 'sessionInfo', id: 'unipi.info-screen/dashboard', owner: 'unipi.info-screen', slot: 'overlay', container: 'overlay', cap: 'chat' },
  { component: 'lhStatus', id: 'unipi.long-horizon/visualize', owner: 'unipi.long-horizon', slot: 'sidePane', container: 'pane', pane: 'git', cap: 'chat' },
  { component: 'turnFiles', id: 'unipi.file-summary/turn-files', owner: 'unipi.file-summary', slot: 'turnFooter', container: 'footer', cap: 'chat' },
];

const byComponent = new Map<string, ReservedRow>(RESERVED_ROWS.map((row) => [row.component, row]));
const byId = new Map<string, ReservedRow>(RESERVED_ROWS.map((row) => [row.id, row]));

/** The row of a wire component name (`sessionInfo`, …). */
export function reservedRowOf(component: string | undefined): ReservedRow | undefined {
  return component ? byComponent.get(component) : undefined;
}

/** The row of a reserved entry id (`unipi.kanboard/board`, …). */
export function reservedRowForId(id: string | undefined): ReservedRow | undefined {
  return id ? byId.get(id) : undefined;
}

/** The row behind a registry entry: its declared component, else its id (§8 declaration). */
export function reservedRowFor(entry: Pick<UiEntry, 'id' | 'view'>): ReservedRow | undefined {
  const declared = entry.view.kind === 'reserved' ? entry.view.component : undefined;
  return reservedRowOf(declared) ?? reservedRowForId(entry.id);
}

/** Is this entry a reserved (native) panel? */
export function isReservedEntry(entry: Pick<UiEntry, 'id' | 'view'>): boolean {
  return reservedRowFor(entry) !== undefined;
}

// ── native surfaces (§10 `reserved` capabilities) ────────────────────────

type Availability = () => boolean;

/**
 * The rows this bundle always renders, gated on the platform (`reserved` in the
 * app's `UiCapabilities`, §10). `turnFiles` joins them with WP-A27's footer body;
 * `ReservedHost` narrows the pane rows further to the pane components that are
 * actually registered (browser in B8, terminal in A29).
 */
const DEFAULT_AVAILABLE: Partial<Record<ReservedComponent, Availability>> = {
  board: () => can('board'),
  cron: () => can('chat'),
  files: () => can('files'),
  git: () => can('files'),
  browser: () => can('hosts'),
  terminal: () => can('terminal'),
  sessionInfo: () => can('chat'),
  lhStatus: () => can('chat'),
  turnFiles: () => can('chat'),
};

const [surfaces, setSurfaces] = createSignal<Partial<Record<ReservedComponent, Availability>>>({});

/**
 * Declares that the app implements one component's native surface, and when
 * that surface is available (`false` ⇒ the owner's fallback tree renders, §8
 * rule 3). Append-only: browser (B8) and terminal (A29) register here when
 * their components land (the turn footer, A27, is in `DEFAULT_AVAILABLE`).
 */
export function registerReservedSurface(component: ReservedComponent, available: Availability): () => void {
  setSurfaces((cur) => ({ ...cur, [component]: available }));
  return () =>
    setSurfaces((cur) => {
      if (cur[component] !== available) return cur;
      const next = { ...cur };
      delete next[component];
      return next;
    });
}

/** Does the app render this component natively right now? */
export function reservedImplemented(component: ReservedComponent): boolean {
  const available = surfaces()[component] ?? DEFAULT_AVAILABLE[component];
  return !!available && available();
}

/** Props of a native reserved body (the entry, its scope and the render mode). */
export type ReservedBodyProps = { entry: UiEntry; target: UiTarget; mode?: Mode };

/** Native bodies of the rows that render *inside* a slot (the phone layer, the footer). */
const bodies = new Map<ReservedComponent, Component<ReservedBodyProps>>();

/**
 * Registers the native component of a row that renders in a slot (Files in the
 * phone's full-screen pane layer, A27's turn footer), append-only like the pane
 * registry. On the desktop the pane/page/overlay containers are used instead;
 * a body only shows where the entry is drawn in place. Returns the unregister.
 */
export function registerReservedBody(component: ReservedComponent, body: Component<ReservedBodyProps>): () => void {
  bodies.set(component, body);
  return () => {
    if (bodies.get(component) === body) bodies.delete(component);
  };
}

/** The registered native body of a component (no owner gate: the caller has the owner's entry). */
export function reservedBodyOf(component: ReservedComponent): Component<ReservedBodyProps> | undefined {
  return bodies.get(component);
}

/** The inline body of an entry's row, when one is registered and the gate is on. */
export function reservedBodyFor(entry: Pick<UiEntry, 'id' | 'view'>): Component<ReservedBodyProps> | undefined {
  const row = reservedRowFor(entry);
  return row && reservedGate(row) ? bodies.get(row.component) : undefined;
}

// ── owner gates (§8 rule 2: entry present ⇔ gate true) ───────────────────

/** Enabled extension ids per host (`_oino/extensions` through `host_cmd`). */
const [owners, setOwners] = createSignal<Record<string, Record<string, boolean>>>({});
/** Host tools per host (`host_info`; `unipi-kanboard` gates the Board row). */
const [tools, setTools] = createSignal<Record<string, { kanboard?: boolean }>>({});
/** Bumped by the Settings toggle so the shell refetches the owner list. */
const [stale, setStale] = createSignal(0);

/** The owner list of `hostId` arrived: extension id → enabled. */
export function setOwnerGates(hostId: string, rows: ReadonlyArray<{ id: string; enabled: boolean }>): void {
  const list: Record<string, boolean> = {};
  for (const row of rows) list[row.id] = row.enabled;
  setOwners((cur) => ({ ...cur, [hostId]: list }));
}

/** `host_info` tools of `hostId` (`unipi_kanboard` present?). */
export function setHostTools(hostId: string, next: { kanboard?: boolean }): void {
  setTools((cur) => ({ ...cur, [hostId]: next }));
}

/** A settings toggle changed an owner: the shell refetches (see `ReservedHost`). */
export function markReservedGatesStale(): void {
  setStale((n) => n + 1);
}

/** Revision of the "gates are stale" signal (the shell's refetch effect). */
export const reservedGatesStale = stale;

/** Tests / a host switch: forget every owner state. */
export function resetReservedGates(): void {
  setOwners({});
  setTools({});
}

/**
 * The gate of one row (§8). `true` only when the app implements the component,
 * the platform can host it, and its owner is enabled. With no host list yet
 * (web build, older host) the platform capability decides, so a surface is
 * never hidden merely because the host could not be asked.
 */
export function reservedGate(rowOrComponent: ReservedRow | ReservedComponent | string | undefined): boolean {
  const row = typeof rowOrComponent === 'object' ? rowOrComponent : reservedRowOf(String(rowOrComponent ?? '')) ?? reservedRowForId(String(rowOrComponent ?? ''));
  if (!row) return false;
  if (!reservedImplemented(row.component)) return false;
  if (row.owner === 'host') return row.cap ? can(row.cap) : true;
  const lists = Object.values(owners());
  const states = lists.map((list) => list[row.owner]).filter((state) => state !== undefined);
  if (!states.length) {
    // Not listed: an owner that is missing from a host's list is not installed
    // there. An empty answer means the host could not tell us (web build, an
    // older host) — then the platform decides, never a silently hidden surface.
    const listable = lists.some((list) => Object.keys(list).length > 0);
    if (!listable) return row.cap ? can(row.cap) : can('chat');
    return false;
  }
  if (!states.some(Boolean)) return false;
  if (row.component === 'board') {
    const cli = Object.values(tools()).map((entry) => entry.kanboard).filter((present) => present !== undefined);
    if (cli.length > 0 && !cli.some(Boolean)) return false; // no `unipi-kanboard` on any host
  }
  return true;
}


/** P1-04: the LH section's own graph inspector asks `/visualize-progress` in place —
 *  that answer must not re-focus (scroll) the status pane each time. */
let lhQuietUntil = 0;
export function quietLhFocus(ms = 4000): void {
  lhQuietUntil = Date.now() + ms;
}
export function lhFocusQuiet(now = Date.now()): boolean {
  return now < lhQuietUntil;
}
