/**
 * The sessions list's view options ([ui-sessions], the Devin-style "Properties"
 * menu): sort, filters, grouping, row property visibility, compact rows and the
 * opt-in subagents tree — persisted per host in app storage
 * (`unipi.sidebar.view.v1`), applied client-side over the host's indexed cards.
 *
 * Everything here is pure except the small persisted store at the bottom; the
 * list renders `viewGroups(…)` and the menu edits `ListView` through
 * `setListView(hostId, patch)`. Built for ~2,000 cards: one linear pass to
 * filter, one sort per group, no per-card allocations beyond the group arrays.
 */
import { createSignal } from 'solid-js';
import type { SessionCard } from '../host/client';
import type { SessionMeta } from './hub';
import { compareCards, isHomeCwd, lastActivity, NO_WORKSPACE_LABEL, sessionCwd, sessionTitle } from './selectors';

export type SortKey = 'updated' | 'created' | 'name';
export type GroupKey = 'none' | 'project' | 'agent' | 'status' | 'automation';
/** A row's status facet (Devin's Status filter, mapped onto unipi's states). */
export type StatusKey = 'running' | 'waiting' | 'ready' | 'failed' | 'timedout';
export type AutomationKey = 'cron' | 'manual';
export type ArchivedKey = 'archived' | 'unarchived';
export type TimePreset = 'today' | 'week' | '2weeks' | 'month' | 'custom';
/** `from`/`to` are local `YYYY-MM-DD` days (inclusive) for `custom`. */
export type TimeRange = { preset: TimePreset; from?: string; to?: string };
export type PropKey = 'updated' | 'status' | 'workspace' | 'model' | 'automation' | 'archived';

export type ListFilters = {
  /** Model ids (`""` = unknown model). Empty = any. */
  models: string[];
  /** Project folders (`""` = No workspace). Empty = any. */
  workspaces: string[];
  automation: AutomationKey[];
  status: StatusKey[];
  created?: TimeRange;
  updated?: TimeRange;
  /** Default `['unarchived']`; both or none = everything. */
  archived: ArchivedKey[];
};

export type ListView = {
  sort: SortKey;
  filters: ListFilters;
  group: GroupKey;
  props: Record<PropKey, boolean>;
  /** One line per row (title + icon chips) instead of title + meta line. */
  compact: boolean;
  /** Every chat with known subagents shows its subagent tree. */
  subagents: boolean;
  /** Chats whose subagent tree was turned on from the row menu. */
  subagentRows: string[];
};

export const DEFAULT_FILTERS: ListFilters = { models: [], workspaces: [], automation: [], status: [], archived: ['unarchived'] };

export const DEFAULT_PROPS: Record<PropKey, boolean> = { updated: true, status: true, workspace: false, model: false, automation: true, archived: true };

export const DEFAULT_VIEW: ListView = {
  sort: 'updated',
  filters: DEFAULT_FILTERS,
  group: 'project',
  props: DEFAULT_PROPS,
  compact: false,
  subagents: false,
  subagentRows: [],
};

export const SORT_LABEL: Record<SortKey, string> = { updated: 'Last updated', created: 'Created', name: 'Name' };
export const GROUP_LABEL: Record<GroupKey, string> = { none: 'None', project: 'Project', agent: 'Agent', status: 'Status', automation: 'Automation' };
export const STATUS_LABEL: Record<StatusKey, string> = { running: 'Running', waiting: 'Waiting for you', ready: 'Ready', failed: 'Failed', timedout: 'Timed out' };
export const STATUS_ORDER: readonly StatusKey[] = ['running', 'waiting', 'ready', 'failed', 'timedout'];
export const AUTOMATION_LABEL: Record<AutomationKey, string> = { cron: 'Automation', manual: 'Not automation' };
export const ARCHIVED_LABEL: Record<ArchivedKey, string> = { archived: 'Archived', unarchived: 'Unarchived' };
export const TIME_LABEL: Record<TimePreset, string> = { today: 'Today', week: 'Last week', '2weeks': 'Last 2 weeks', month: 'Last month', custom: 'Custom…' };
export const PROP_LABEL: Record<PropKey, string> = { updated: 'Last updated', status: 'Status', workspace: 'Workspace', model: 'Model', automation: 'Automation', archived: 'Archived' };
export const PROP_ORDER: readonly PropKey[] = ['updated', 'status', 'workspace', 'model', 'automation', 'archived'];
export const UNKNOWN_MODEL_LABEL = 'Unknown model';

// ── card facets ─────────────────────────────────────────────────────────

/** `<ISO ts with - for : and .>_<id>.jsonl` (oino-session file names) → epoch ms. */
export function createdFromFile(file: string | undefined): number | undefined {
  if (!file) return undefined;
  const name = file.split(/[\\/]/).pop() ?? '';
  const m = /^(\d{4}-\d{2}-\d{2})T(\d{2})-(\d{2})-(\d{2})-(\d{3})Z_/.exec(name);
  if (!m) return undefined;
  const ms = Date.parse(`${m[1]}T${m[2]}:${m[3]}:${m[4]}.${m[5]}Z`);
  return Number.isFinite(ms) ? ms : undefined;
}

/** When the chat was created: the host's `created_at`, else the file name's stamp, else its last activity. */
export function createdAt(card: SessionCard): number {
  const c = card as SessionCard & { created_at?: unknown };
  if (typeof c.created_at === 'number' && c.created_at > 0) return c.created_at;
  return createdFromFile(card.file) ?? lastActivity(card);
}

/** The card's project folder key (`""` = No workspace, D-18). */
export function workspaceKey(card: SessionCard): string {
  const cwd = sessionCwd(card);
  return isHomeCwd(cwd) ? '' : cwd;
}

export const workspaceLabel = (key: string): string => (key ? key.replace(/[\\/]+$/, '').split(/[\\/]/).pop() || key : NO_WORKSPACE_LABEL);

/** The model id the card names (`provider/id` → `id`; `""` = unknown). */
export function modelKey(card: SessionCard, live?: string): string {
  const raw = (live ?? card.model ?? '').trim();
  return raw;
}

export const modelLabel = (key: string): string => (key ? (key.includes('/') ? key.split('/').pop()! : key) : UNKNOWN_MODEL_LABEL);

export const isAutomation = (card: SessionCard): boolean => card.source === 'cron';

/** The facts a filter needs that the card alone does not carry (live state, cron outcome). */
export type CardFacts = {
  /** Live row state (`liveRowState`). */
  state: 'needs-you' | 'working' | 'crashed' | 'taken-over' | 'background' | 'idle';
  /** A cron run's outcome (`cron_runs` status). */
  cronStatus?: string;
  /** The open session's model, when it is open. */
  model?: string;
  archived?: boolean;
};

export function statusOf(facts: Pick<CardFacts, 'state' | 'cronStatus'>): StatusKey {
  if (facts.state === 'needs-you') return 'waiting';
  if (facts.state === 'working') return 'running';
  if (facts.cronStatus === 'timeout') return 'timedout';
  if (facts.state === 'crashed' || facts.cronStatus === 'failed') return 'failed';
  return 'ready';
}

// ── time ranges ─────────────────────────────────────────────────────────

const DAY = 86_400_000;

/** `YYYY-MM-DD` (local) → the start of that day, epoch ms. */
export function dayStart(day: string): number | undefined {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return undefined;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).getTime();
}

/** Local `YYYY-MM-DD` of an epoch ms. */
export function dayOf(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

/** `[from, to)` epoch ms of a range (open ends are ±∞). */
export function rangeBounds(range: TimeRange, now = Date.now()): [number, number] {
  const today = dayStart(dayOf(now))!;
  switch (range.preset) {
    case 'today':
      return [today, Infinity];
    case 'week':
      return [now - 7 * DAY, Infinity];
    case '2weeks':
      return [now - 14 * DAY, Infinity];
    case 'month':
      return [now - 30 * DAY, Infinity];
    case 'custom': {
      const from = range.from ? dayStart(range.from) : undefined;
      const to = range.to ? dayStart(range.to) : undefined;
      return [from ?? -Infinity, to !== undefined ? to + DAY : Infinity];
    }
  }
}

export function rangeLabel(range: TimeRange | undefined): string | undefined {
  if (!range) return undefined;
  if (range.preset !== 'custom') return TIME_LABEL[range.preset];
  if (range.from && range.to) return `${range.from} – ${range.to}`;
  if (range.from) return `Since ${range.from}`;
  if (range.to) return `Until ${range.to}`;
  return 'Custom';
}

const inRange = (ms: number, range: TimeRange | undefined, now: number): boolean => {
  if (!range) return true;
  const [from, to] = rangeBounds(range, now);
  return ms >= from && ms < to;
};

// ── filters ─────────────────────────────────────────────────────────────

export type FilterFacet = 'models' | 'workspaces' | 'automation' | 'status' | 'created' | 'updated' | 'archived';

/** Facets that differ from the default (each counts once, like Devin's "Filter · 2"). */
export function activeFacets(f: ListFilters): FilterFacet[] {
  const out: FilterFacet[] = [];
  if (f.models.length) out.push('models');
  if (f.workspaces.length) out.push('workspaces');
  if (f.automation.length) out.push('automation');
  if (f.status.length) out.push('status');
  if (f.created) out.push('created');
  if (f.updated) out.push('updated');
  if (!(f.archived.length === 1 && f.archived[0] === 'unarchived')) out.push('archived');
  return out;
}

export const activeFilterCount = (f: ListFilters): number => activeFacets(f).length;

/** The archived facet asks for the host's trash too. */
export const wantsArchived = (f: ListFilters): boolean => f.archived.length === 0 || f.archived.includes('archived');
/** The archived facet keeps live (unarchived) chats. */
export const wantsLive = (f: ListFilters): boolean => f.archived.length === 0 || f.archived.includes('unarchived');

export function matchCard(card: SessionCard, facts: CardFacts, f: ListFilters, now = Date.now()): boolean {
  if (f.archived.length === 1 && (f.archived[0] === 'archived') !== !!facts.archived) return false;
  if (f.models.length && !f.models.includes(modelKey(card, facts.model))) return false;
  if (f.workspaces.length && !f.workspaces.includes(workspaceKey(card))) return false;
  if (f.automation.length && !f.automation.includes(isAutomation(card) ? 'cron' : 'manual')) return false;
  if (f.status.length && !f.status.includes(statusOf(facts))) return false;
  if (f.created && !inRange(createdAt(card), f.created, now)) return false;
  if (f.updated && !inRange(lastActivity(card), f.updated, now)) return false;
  return true;
}

// ── sorting + grouping ──────────────────────────────────────────────────

/** Row order inside a group. `updated` keeps Z-9's pinned → running → needs-you → recent. */
export function sortCards(cards: SessionCard[], sort: SortKey, meta: Readonly<Record<string, SessionMeta>> = {}): SessionCard[] {
  if (sort === 'updated') return cards.sort((a, b) => compareCards(a, b, meta));
  if (sort === 'created') return cards.sort((a, b) => createdAt(b) - createdAt(a) || a.session_id.localeCompare(b.session_id));
  const titles = new Map(cards.map((c) => [c.session_id, sessionTitle(c, undefined, 'New chat').toLocaleLowerCase()]));
  return cards.sort((a, b) => titles.get(a.session_id)!.localeCompare(titles.get(b.session_id)!) || a.session_id.localeCompare(b.session_id));
}

export type ViewGroup = {
  /** Stable key (`project:<cwd>`, `status:running`, `all`). */
  key: string;
  label: string;
  /** Project groups: the folder (`""` = No workspace) — drives the `+` draft and the tooltip. */
  cwd?: string;
  cards: SessionCard[];
};

export type ViewGroups = { pinned: SessionCard[]; groups: ViewGroup[]; total: number };

/**
 * The list a host shows under `view`: filtered cards, the Pinned group on top
 * (Z-14, still filtered), then the groups of the chosen grouping, each sorted.
 * `factsOf` supplies the live state / cron outcome / model of a card.
 */
export function viewGroups(
  cards: readonly SessionCard[],
  meta: Readonly<Record<string, SessionMeta>>,
  view: ListView,
  factsOf: (card: SessionCard) => CardFacts,
  now = Date.now(),
): ViewGroups {
  const pinned: SessionCard[] = [];
  const byKey = new Map<string, ViewGroup & { order: number }>();
  let total = 0;
  for (const card of cards) {
    const facts = factsOf(card);
    if (!matchCard(card, facts, view.filters, now)) continue;
    total += 1;
    if (!facts.archived && meta[card.session_id]?.pinned) {
      pinned.push(card);
      continue;
    }
    let key: string;
    let label: string;
    let cwd: string | undefined;
    let order = 0;
    switch (view.group) {
      case 'none':
        key = 'all';
        label = 'Sessions';
        break;
      case 'project':
        cwd = workspaceKey(card);
        key = `project:${cwd}`;
        label = workspaceLabel(cwd);
        break;
      case 'agent': {
        const m = modelKey(card, facts.model);
        key = `agent:${m}`;
        label = modelLabel(m);
        order = m ? 0 : 1;
        break;
      }
      case 'status': {
        const s = statusOf(facts);
        key = `status:${s}`;
        label = STATUS_LABEL[s];
        order = STATUS_ORDER.indexOf(s);
        break;
      }
      case 'automation': {
        const a: AutomationKey = isAutomation(card) ? 'cron' : 'manual';
        key = `automation:${a}`;
        label = AUTOMATION_LABEL[a];
        order = a === 'cron' ? 0 : 1;
        break;
      }
    }
    let g = byKey.get(key);
    if (!g) {
      g = { key, label, ...(cwd !== undefined ? { cwd } : {}), cards: [], order };
      byKey.set(key, g);
    }
    g.cards.push(card);
  }
  const groups = [...byKey.values()];
  for (const g of groups) sortCards(g.cards, view.sort, meta);
  sortCards(pinned, view.sort === 'updated' ? 'updated' : view.sort, {});
  // Groups: fixed facets keep their order; projects / agents follow the sort.
  const head = (g: ViewGroup) => g.cards[0];
  groups.sort((a, b) => {
    if (view.group === 'status' || view.group === 'automation') return a.order - b.order;
    if (a.order !== b.order) return a.order - b.order;
    if (view.sort === 'name') return a.label.localeCompare(b.label);
    if (view.sort === 'created') return Math.max(...b.cards.map(createdAt)) - Math.max(...a.cards.map(createdAt));
    return maxActivity(b.cards) - maxActivity(a.cards) || a.label.localeCompare(b.label) || (head(a)?.session_id ?? '').localeCompare(head(b)?.session_id ?? '');
  });
  return { pinned, groups: groups.map(({ order: _o, ...g }) => g), total };
}

function maxActivity(cards: readonly SessionCard[]): number {
  let max = 0;
  for (const c of cards) max = Math.max(max, lastActivity(c));
  return max;
}

/** The option lists a facet offers, with counts over the (unfiltered) live cards. */
export function facetOptions(cards: readonly SessionCard[], key: (c: SessionCard) => string): { key: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const c of cards) {
    const k = key(c);
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  return [...counts.entries()].map(([k, count]) => ({ key: k, count })).sort((a, b) => b.count - a.count || a.key.localeCompare(b.key));
}

// ── the persisted per-host store ────────────────────────────────────────

export const LIST_VIEW_KEY = 'unipi.sidebar.view.v1';

/** Fills missing / malformed fields from the defaults (older or hand-edited storage). */
export function normalizeView(raw: unknown): ListView {
  const v = (raw && typeof raw === 'object' ? raw : {}) as Partial<ListView>;
  const f = (v.filters && typeof v.filters === 'object' ? v.filters : {}) as Partial<ListFilters>;
  const strs = (x: unknown): string[] => (Array.isArray(x) ? x.filter((s): s is string => typeof s === 'string') : []);
  const pick = <T extends string>(x: unknown, allowed: readonly T[], fallback: T): T => (allowed.includes(x as T) ? (x as T) : fallback);
  const range = (x: unknown): TimeRange | undefined => {
    const r = x as Partial<TimeRange> | undefined;
    if (!r || typeof r !== 'object' || !(r.preset && r.preset in TIME_LABEL)) return undefined;
    return { preset: r.preset, ...(typeof r.from === 'string' ? { from: r.from } : {}), ...(typeof r.to === 'string' ? { to: r.to } : {}) };
  };
  const props = { ...DEFAULT_PROPS };
  if (v.props && typeof v.props === 'object') for (const k of PROP_ORDER) if (typeof v.props[k] === 'boolean') props[k] = v.props[k];
  const created = range(f.created);
  const updated = range(f.updated);
  return {
    sort: pick(v.sort, ['updated', 'created', 'name'] as const, 'updated'),
    group: pick(v.group, ['none', 'project', 'agent', 'status', 'automation'] as const, 'project'),
    filters: {
      models: strs(f.models),
      workspaces: strs(f.workspaces),
      automation: strs(f.automation).filter((s): s is AutomationKey => s === 'cron' || s === 'manual'),
      status: strs(f.status).filter((s): s is StatusKey => (STATUS_ORDER as readonly string[]).includes(s)),
      ...(created ? { created } : {}),
      ...(updated ? { updated } : {}),
      archived: Array.isArray(f.archived) ? strs(f.archived).filter((s): s is ArchivedKey => s === 'archived' || s === 'unarchived') : ['unarchived'],
    },
    props,
    compact: v.compact === true,
    subagents: v.subagents === true,
    subagentRows: strs(v.subagentRows).slice(-200),
  };
}

function readViews(): Record<string, ListView> {
  try {
    const raw = JSON.parse(globalThis.localStorage?.getItem(LIST_VIEW_KEY) ?? '{}') as Record<string, unknown>;
    if (!raw || typeof raw !== 'object') return {};
    return Object.fromEntries(Object.entries(raw).map(([k, v]) => [k, normalizeView(v)]));
  } catch {
    return {};
  }
}

const [views, setViews] = createSignal<Record<string, ListView>>(readViews());

/** The host's list view (reactive). */
export function listView(hostId: string): ListView {
  return views()[hostId] ?? DEFAULT_VIEW;
}

type ViewPatch = Partial<Omit<ListView, 'filters' | 'props'>> & { filters?: Partial<ListFilters>; props?: Partial<Record<PropKey, boolean>> };

export function setListView(hostId: string, patch: ViewPatch): void {
  setViews((cur) => {
    const prev = cur[hostId] ?? DEFAULT_VIEW;
    const filters = { ...prev.filters, ...patch.filters } as ListFilters;
    // `undefined` clears a time range.
    if (patch.filters && 'created' in patch.filters && !patch.filters.created) delete filters.created;
    if (patch.filters && 'updated' in patch.filters && !patch.filters.updated) delete filters.updated;
    const next: ListView = { ...prev, ...patch, filters, props: { ...prev.props, ...patch.props } } as ListView;
    const all = { ...cur, [hostId]: next };
    try {
      globalThis.localStorage?.setItem(LIST_VIEW_KEY, JSON.stringify(all));
    } catch {
      // not remembered
    }
    return all;
  });
}

export function resetListFilters(hostId: string): void {
  setListView(hostId, { filters: { ...DEFAULT_FILTERS, created: undefined, updated: undefined } });
}

/** Toggles one value in a multi-select facet. */
export function toggleIn<T>(list: readonly T[], value: T): T[] {
  return list.includes(value) ? list.filter((v) => v !== value) : [...list, value];
}

/** Turns one chat's subagent tree on/off (the row menu's "Show subagents"). */
export function toggleSubagentRow(hostId: string, sessionId: string): void {
  setListView(hostId, { subagentRows: toggleIn(listView(hostId).subagentRows, sessionId) });
}

/** Test support: forget every host's view. */
export function resetListViews(): void {
  setViews({});
  try {
    globalThis.localStorage?.removeItem(LIST_VIEW_KEY);
  } catch {
    // ignore
  }
}
