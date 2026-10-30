/**
 * Side pane + bottom panel tab state (app spec §3.3 "panes", §3.8, WP-A3; Z-5).
 *
 * Tabs live per **project scope** `(hostId, projectKey)`; kinds whose scope
 * is `session` are tagged with the `sessionId` they were opened from, hidden
 * while another session is shown and back when it returns (ZCode
 * `side-pane-tabs.md`: workspace-public tabs vs. owner-tagged tabs). Close /
 * close others / close all / reopen only touch the tabs visible in the
 * current view; recently closed keeps 10 per scope; the user's order is the
 * array order. Persisted per project in localStorage; session-scoped tabs are
 * dropped with their session (`dropSession`).
 *
 * The pure functions below take a `KindRule` (singleton / scope) so they are
 * testable without the kind registry; `panes()` is the app store that reads
 * the rules from `lib/panes/kinds.ts`.
 */
import { createRoot, createSignal } from 'solid-js';

export type PaneArea = 'side' | 'bottom';

export type PaneTab = {
  id: string;
  kind: string;
  params?: Record<string, unknown>;
  /** A title the tab set itself (e.g. a page title); else the kind's title. */
  title?: string;
  /** Session-scoped tabs: the session they belong to. */
  sessionId?: string;
};

export type PaneSet = {
  tabs: PaneTab[];
  /** Active tab per view (`sessionId` or `''`). */
  active: Record<string, string>;
  /** Recently closed, newest first (≤ RECENT_MAX). */
  closed: PaneTab[];
};

export type PaneView = { sessionId?: string };

export type KindRule = {
  /** `true`: one per scope; a function: one per key (file, path, agent, entry); `false`: many. */
  singleton: boolean | ((params: Record<string, unknown>) => string);
  scope: 'project' | 'session';
  /** Kept out of "recently closed" (e.g. transient tabs). */
  noRecent?: boolean;
};

export const RECENT_MAX = 10;
export const EMPTY_SET: PaneSet = { tabs: [], active: {}, closed: [] };

const viewKey = (view: PaneView): string => view.sessionId ?? '';

/** Pure: the tabs the current view shows, in the user's order. */
export function visibleTabs(set: PaneSet, view: PaneView): PaneTab[] {
  return set.tabs.filter((t) => !t.sessionId || t.sessionId === view.sessionId);
}

/** Pure: the active tab of this view (falls back to the first visible tab). */
export function activeTab(set: PaneSet, view: PaneView): PaneTab | undefined {
  const tabs = visibleTabs(set, view);
  const id = set.active[viewKey(view)];
  return tabs.find((t) => t.id === id) ?? tabs[0];
}

function instanceKey(rule: KindRule, params: Record<string, unknown>): string | undefined {
  if (rule.singleton === true) return '';
  if (typeof rule.singleton === 'function') return rule.singleton(params);
  return undefined;
}

function withActive(set: PaneSet, view: PaneView, id: string | undefined): PaneSet {
  const active = { ...set.active };
  if (id) active[viewKey(view)] = id;
  else delete active[viewKey(view)];
  return { ...set, active };
}

/**
 * Pure: opens a tab (or activates the existing instance of a singleton /
 * per-key kind visible in this view). Returns the new set and the tab id.
 */
export function openTab(
  set: PaneSet,
  spec: { kind: string; params?: Record<string, unknown>; title?: string },
  view: PaneView,
  rule: KindRule,
  newId: () => string,
): { set: PaneSet; id: string } {
  const params = spec.params ?? {};
  const key = instanceKey(rule, params);
  if (key !== undefined) {
    const hit = visibleTabs(set, view).find((t) => t.kind === spec.kind && instanceKey(rule, t.params ?? {}) === key);
    if (hit) return { set: withActive(set, view, hit.id), id: hit.id };
  }
  const tab: PaneTab = { id: newId(), kind: spec.kind };
  if (spec.params) tab.params = spec.params;
  if (spec.title) tab.title = spec.title;
  if (rule.scope === 'session' && view.sessionId) tab.sessionId = view.sessionId;
  return { set: withActive({ ...set, tabs: [...set.tabs, tab] }, view, tab.id), id: tab.id };
}

/** Pure: activates a visible tab. */
export function activateTab(set: PaneSet, id: string, view: PaneView): PaneSet {
  return visibleTabs(set, view).some((t) => t.id === id) ? withActive(set, view, id) : set;
}

function remember(closed: PaneTab[], tabs: PaneTab[], rules: (kind: string) => KindRule | undefined): PaneTab[] {
  const kept = tabs.filter((t) => !rules(t.kind)?.noRecent);
  const ids = new Set(kept.map((t) => t.id));
  return [...kept.reverse(), ...closed.filter((t) => !ids.has(t.id))].slice(0, RECENT_MAX);
}

function removeTabs(set: PaneSet, ids: Set<string>, view: PaneView, rules: (kind: string) => KindRule | undefined): PaneSet {
  const visible = visibleTabs(set, view);
  const current = activeTab(set, view);
  const gone = set.tabs.filter((t) => ids.has(t.id));
  let next: PaneSet = { ...set, tabs: set.tabs.filter((t) => !ids.has(t.id)), closed: remember(set.closed, gone, rules) };
  if (current && ids.has(current.id)) {
    // ZCode: the right neighbour, else the left one, among this view's tabs.
    const at = visible.findIndex((t) => t.id === current.id);
    const right = visible.slice(at + 1).find((t) => !ids.has(t.id));
    const left = visible.slice(0, at).reverse().find((t) => !ids.has(t.id));
    next = withActive(next, view, (right ?? left)?.id);
  }
  return next;
}

/** Pure: closes one visible tab (it goes to recently closed). */
export function closeTab(set: PaneSet, id: string, view: PaneView, rules: (kind: string) => KindRule | undefined = () => undefined): PaneSet {
  if (!visibleTabs(set, view).some((t) => t.id === id)) return set;
  return removeTabs(set, new Set([id]), view, rules);
}

/** Pure: closes every visible tab but `id`. */
export function closeOtherTabs(set: PaneSet, id: string, view: PaneView, rules: (kind: string) => KindRule | undefined = () => undefined): PaneSet {
  const ids = new Set(visibleTabs(set, view).filter((t) => t.id !== id).map((t) => t.id));
  return withActive(removeTabs(set, ids, view, rules), view, id);
}

/** Pure: closes every visible tab (hidden tabs of other sessions stay). */
export function closeAllTabs(set: PaneSet, view: PaneView, rules: (kind: string) => KindRule | undefined = () => undefined): PaneSet {
  return removeTabs(set, new Set(visibleTabs(set, view).map((t) => t.id)), view, rules);
}

/** Pure: recently closed tabs this view can restore (project tabs anywhere, session tabs in their session). */
export function recentlyClosed(set: PaneSet, view: PaneView): PaneTab[] {
  return set.closed.filter((t) => !t.sessionId || t.sessionId === view.sessionId);
}

/** Pure: reopens a recently closed tab (the newest restorable one by default) with its id and owner. */
export function reopenTab(set: PaneSet, view: PaneView, id?: string): { set: PaneSet; id?: string } {
  const tab = recentlyClosed(set, view).find((t) => id === undefined || t.id === id);
  if (!tab) return { set };
  const tabs = set.tabs.some((t) => t.id === tab.id) ? set.tabs : [...set.tabs, tab];
  return { set: withActive({ ...set, tabs, closed: set.closed.filter((t) => t.id !== tab.id) }, view, tab.id), id: tab.id };
}

/** Pure: moves `id` before `beforeId` (`undefined` = to the end). The order is the user's. */
export function moveTab(set: PaneSet, id: string, beforeId?: string): PaneSet {
  if (id === beforeId) return set;
  const tab = set.tabs.find((t) => t.id === id);
  if (!tab) return set;
  const rest = set.tabs.filter((t) => t.id !== id);
  const at = beforeId ? rest.findIndex((t) => t.id === beforeId) : -1;
  const tabs = at < 0 ? [...rest, tab] : [...rest.slice(0, at), tab, ...rest.slice(at)];
  return { ...set, tabs };
}

/** Pure: forgets a session's tabs (open and recently closed) and its active entry. */
export function dropSessionTabs(set: PaneSet, sessionId: string): PaneSet {
  const active = { ...set.active };
  delete active[sessionId];
  return { tabs: set.tabs.filter((t) => t.sessionId !== sessionId), active, closed: set.closed.filter((t) => t.sessionId !== sessionId) };
}

/** Pure: retitles a tab. */
export function retitleTab(set: PaneSet, id: string, title: string | undefined): PaneSet {
  return { ...set, tabs: set.tabs.map((t) => (t.id === id ? { ...t, title } : t)) };
}

// ── The app store ───────────────────────────────────────────────────────

export type ScopePanes = Record<PaneArea, PaneSet>;
export type PaneScope = { hostId: string; projectKey: string };

export const scopeKey = (scope: PaneScope): string => `${scope.hostId}|${scope.projectKey}`;
const STORE_PREFIX = 'unipi.panes:';

function parseScope(raw: string | null): ScopePanes {
  const empty: ScopePanes = { side: EMPTY_SET, bottom: EMPTY_SET };
  if (!raw) return empty;
  try {
    const parsed = JSON.parse(raw) as Partial<ScopePanes>;
    const fix = (s: Partial<PaneSet> | undefined): PaneSet => ({
      tabs: Array.isArray(s?.tabs) ? s.tabs.filter((t) => t && typeof t.id === 'string' && typeof t.kind === 'string') : [],
      active: s?.active && typeof s.active === 'object' ? s.active : {},
      closed: Array.isArray(s?.closed) ? s.closed.slice(0, RECENT_MAX) : [],
    });
    return { side: fix(parsed.side), bottom: fix(parsed.bottom) };
  } catch {
    return empty;
  }
}

export type PanesStoreOptions = {
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> & Partial<Pick<Storage, 'key' | 'length'>>;
  rules: (kind: string) => KindRule | undefined;
  newId?: () => string;
};

/** The panes store (one per app; `panes()`); every op is keyed by scope + area + view. */
export function createPanes(opts: PanesStoreOptions) {
  const [all, setAll] = createSignal<Record<string, ScopePanes>>({});
  let counter = 0;
  const newId = opts.newId ?? (() => `t${Date.now().toString(36)}${(counter++).toString(36)}`);
  const rule = (kind: string): KindRule => opts.rules(kind) ?? { singleton: false, scope: 'project' };

  const load = (key: string): ScopePanes => {
    const hit = all()[key];
    if (hit) return hit;
    let raw: string | null = null;
    try {
      raw = opts.storage?.getItem(STORE_PREFIX + key) ?? null;
    } catch {
      raw = null;
    }
    return parseScope(raw);
  };
  const write = (key: string, next: ScopePanes) => {
    setAll((cur) => ({ ...cur, [key]: next }));
    try {
      const empty = next.side.tabs.length + next.bottom.tabs.length + next.side.closed.length + next.bottom.closed.length === 0;
      if (empty) opts.storage?.removeItem(STORE_PREFIX + key);
      else opts.storage?.setItem(STORE_PREFIX + key, JSON.stringify(next));
    } catch {
      // storage full / blocked: tabs just aren't remembered
    }
  };
  const update = (scope: PaneScope, area: PaneArea, fn: (set: PaneSet) => PaneSet) => {
    const key = scopeKey(scope);
    const cur = load(key);
    const next = fn(cur[area]);
    if (next !== cur[area]) write(key, { ...cur, [area]: next });
  };

  return {
    /** The tab set of a scope + area (reactive). */
    set(scope: PaneScope, area: PaneArea): PaneSet {
      return load(scopeKey(scope))[area];
    },
    visible(scope: PaneScope, area: PaneArea, view: PaneView): PaneTab[] {
      return visibleTabs(load(scopeKey(scope))[area], view);
    },
    active(scope: PaneScope, area: PaneArea, view: PaneView): PaneTab | undefined {
      return activeTab(load(scopeKey(scope))[area], view);
    },
    recentlyClosed(scope: PaneScope, area: PaneArea, view: PaneView): PaneTab[] {
      return recentlyClosed(load(scopeKey(scope))[area], view);
    },
    open(scope: PaneScope, area: PaneArea, view: PaneView, spec: { kind: string; params?: Record<string, unknown>; title?: string }): string {
      let id = '';
      update(scope, area, (set) => {
        const r = openTab(set, spec, view, rule(spec.kind), newId);
        id = r.id;
        return r.set;
      });
      return id;
    },
    activate(scope: PaneScope, area: PaneArea, view: PaneView, id: string): void {
      update(scope, area, (set) => activateTab(set, id, view));
    },
    close(scope: PaneScope, area: PaneArea, view: PaneView, id: string): void {
      update(scope, area, (set) => closeTab(set, id, view, opts.rules));
    },
    closeOthers(scope: PaneScope, area: PaneArea, view: PaneView, id: string): void {
      update(scope, area, (set) => closeOtherTabs(set, id, view, opts.rules));
    },
    closeAll(scope: PaneScope, area: PaneArea, view: PaneView): void {
      update(scope, area, (set) => closeAllTabs(set, view, opts.rules));
    },
    reopen(scope: PaneScope, area: PaneArea, view: PaneView, id?: string): string | undefined {
      let out: string | undefined;
      update(scope, area, (set) => {
        const r = reopenTab(set, view, id);
        out = r.id;
        return r.set;
      });
      return out;
    },
    move(scope: PaneScope, area: PaneArea, id: string, beforeId?: string): void {
      update(scope, area, (set) => moveTab(set, id, beforeId));
    },
    retitle(scope: PaneScope, area: PaneArea, id: string, title: string | undefined): void {
      update(scope, area, (set) => retitleTab(set, id, title));
    },
    /** Session gone (deleted / archived): drop its tabs in every loaded scope of that host. */
    dropSession(hostId: string, sessionId: string): void {
      const keys = new Set(Object.keys(all()));
      const st = opts.storage;
      if (st?.key && typeof st.length === 'number') {
        for (let i = 0; i < st.length; i++) {
          const k = st.key(i);
          if (k?.startsWith(STORE_PREFIX)) keys.add(k.slice(STORE_PREFIX.length));
        }
      }
      for (const key of keys) {
        if (!key.startsWith(`${hostId}|`)) continue;
        const cur = load(key);
        const next = { side: dropSessionTabs(cur.side, sessionId), bottom: dropSessionTabs(cur.bottom, sessionId) };
        if (next.side.tabs.length !== cur.side.tabs.length || next.bottom.tabs.length !== cur.bottom.tabs.length || next.side.closed.length !== cur.side.closed.length || next.bottom.closed.length !== cur.bottom.closed.length) write(key, next);
      }
    },
  };
}

export type PanesStore = ReturnType<typeof createPanes>;

let shared: PanesStore | undefined;
let rulesSource: (kind: string) => KindRule | undefined = () => undefined;

/** Wires the kind registry in (lib/panes/kinds.ts does this on import). */
export function setPaneRules(rules: (kind: string) => KindRule | undefined): void {
  rulesSource = rules;
}

/** The app's panes store. */
export function panes(): PanesStore {
  if (!shared) {
    const store = (() => {
      try {
        return typeof localStorage === 'undefined' ? undefined : localStorage;
      } catch {
        return undefined;
      }
    })();
    shared = createRoot(() => createPanes({ storage: store, rules: (kind) => rulesSource(kind) }));
  }
  return shared;
}
/** Test hook. */
export function resetPanes(): void {
  shared = undefined;
}
