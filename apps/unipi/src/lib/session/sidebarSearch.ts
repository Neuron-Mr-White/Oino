/**
 * [ux-sessions] The sidebar's session search (owner fix 2026-11-03): one query box in the
 * "Task list" header that asks the HOST (`sessions_query`) — title + first user message over
 * every chat, live and archived (cold ones too: the host reads its card indexes), paged, with
 * the list's Properties filters applied host-side. The webview only ever holds the pages the
 * user asked for (at most {@link SEARCH_MAX_ROWS} rows).
 *
 *   openSidebarSearch() · sidebarSearch.text() · sidebarSearch.query() (debounced) · clearSidebarSearch()
 *   const pages = createSearchPages(hostId, () => searchArgs(view, query))
 */
import { createEffect, createSignal, on, onCleanup } from 'solid-js';
import { SESSIONS_QUERY_LIMIT_DEFAULT, toAppError, type AppError, type SessionsQueryHit, type SessionsQueryScope } from '@unipi/protocol';
import { hostClient } from '../host/factory';
import { rangeBounds, type ListView } from './listView';

/** Typing settles this long before the host is asked. */
export const SEARCH_DEBOUNCE_MS = 200;
/** Rows one host's results may grow to by "Show more" (then: refine the query). */
export const SEARCH_MAX_ROWS = 500;

const [open, setOpen] = createSignal(false);
const [text, setTextNow] = createSignal('');
const [query, setQuery] = createSignal('');
/** Bumped by `/` / Ctrl+F so the header focuses its input. */
const [focusTick, setFocusTick] = createSignal(0);
let debounce: ReturnType<typeof setTimeout> | undefined;

export const sidebarSearch = {
  /** The search row is shown. */
  open,
  /** What the box shows (immediate). */
  text,
  /** The settled query the results follow (debounced, trimmed). */
  query,
  focusTick,
};

/** Types into the search box; the results follow after {@link SEARCH_DEBOUNCE_MS}. */
export function setSidebarSearchText(value: string): void {
  setTextNow(value);
  if (debounce !== undefined) clearTimeout(debounce);
  const next = value.trim();
  if (!next) {
    debounce = undefined;
    setQuery('');
    return;
  }
  debounce = setTimeout(() => {
    debounce = undefined;
    setQuery(next);
  }, SEARCH_DEBOUNCE_MS);
}

/** Shows the search row and focuses it (the header's icon, `/`, Ctrl+F). */
export function openSidebarSearch(): void {
  setOpen(true);
  setFocusTick((n) => n + 1);
}

/** Esc: clears the query and hides the row. */
export function clearSidebarSearch(): void {
  if (debounce !== undefined) clearTimeout(debounce);
  debounce = undefined;
  setTextNow('');
  setQuery('');
  setOpen(false);
}

/** `sessions_query` arguments (minus paging) for a host's view and a query. */
export type SearchArgs = {
  query: string;
  scope: SessionsQueryScope;
  sort: ListView['sort'];
  filters: {
    workspaces?: string[];
    automation?: ('cron' | 'manual')[];
    status?: ('running' | 'waiting' | 'ready' | 'failed' | 'timedout')[];
    created?: { from?: number; to?: number };
    updated?: { from?: number; to?: number };
  };
};

const bounds = (range: Parameters<typeof rangeBounds>[0] | undefined, now: number) => {
  if (!range) return undefined;
  const [from, to] = rangeBounds(range, now);
  return { ...(Number.isFinite(from) ? { from: Math.floor(from) } : {}), ...(Number.isFinite(to) ? { to: Math.floor(to) } : {}) };
};

/**
 * Pure: the host query for `view` + `query`. Search always reaches the archived chats (the owner
 * asked for it): the default visibility (Unarchived) searches everything and marks archived hits;
 * an explicit "Archived" only searches the trash. Models stay a client-side facet (the host does
 * not know a chat's model before it opens).
 */
export function searchArgs(view: ListView, query: string, now = Date.now()): SearchArgs {
  const f = view.filters;
  const scope: SessionsQueryScope = f.archived.length === 1 && f.archived[0] === 'archived' ? 'archived' : 'all';
  const created = bounds(f.created, now);
  const updated = bounds(f.updated, now);
  return {
    query,
    scope,
    sort: view.sort,
    filters: {
      ...(f.workspaces.length ? { workspaces: f.workspaces } : {}),
      ...(f.automation.length ? { automation: f.automation } : {}),
      ...(f.status.length ? { status: f.status } : {}),
      ...(created ? { created } : {}),
      ...(updated ? { updated } : {}),
    },
  };
}

export type SearchPages = {
  hits: () => SessionsQueryHit[];
  total: () => number;
  /** First page in flight (skeleton). */
  loading: () => boolean;
  /** A next page in flight. */
  loadingMore: () => boolean;
  error: () => AppError | undefined;
  hasMore: () => boolean;
  /** At {@link SEARCH_MAX_ROWS}: no more paging, refine the query. */
  capped: () => boolean;
  more: () => void;
  retry: () => void;
};

/**
 * One host's result pages for `args()` (`undefined` = no search): the first page whenever the
 * args change (stale replies are dropped), "Show more" appends the next one.
 */
export function createSearchPages(hostId: () => string, args: () => SearchArgs | undefined): SearchPages {
  const [hits, setHits] = createSignal<SessionsQueryHit[]>([]);
  const [total, setTotal] = createSignal(0);
  const [cursor, setCursor] = createSignal<string | undefined>();
  const [loading, setLoading] = createSignal(false);
  const [loadingMore, setLoadingMore] = createSignal(false);
  const [error, setError] = createSignal<AppError | undefined>();
  let seq = 0;

  const fetchPage = async (a: SearchArgs, at: string | undefined, append: boolean) => {
    const mine = ++seq;
    (append ? setLoadingMore : setLoading)(true);
    setError(undefined);
    try {
      const data = await hostClient(hostId()).request<{ hits?: SessionsQueryHit[]; total?: number; next_cursor?: string }>('sessions_query', {
        query: a.query,
        scope: a.scope,
        sort: a.sort,
        filters: a.filters,
        limit: SESSIONS_QUERY_LIMIT_DEFAULT,
        ...(at ? { cursor: at } : {}),
      });
      if (mine !== seq) return;
      const page = Array.isArray(data?.hits) ? data.hits.filter((h) => h && typeof h.card?.session_id === 'string') : [];
      setHits((cur) => (append ? [...cur, ...page].slice(0, SEARCH_MAX_ROWS) : page.slice(0, SEARCH_MAX_ROWS)));
      setTotal(typeof data?.total === 'number' ? data.total : page.length);
      setCursor(typeof data?.next_cursor === 'string' ? data.next_cursor : undefined);
    } catch (e) {
      if (mine !== seq) return;
      setError(toAppError(e));
      if (!append) setHits([]);
    } finally {
      if (mine === seq) (append ? setLoadingMore : setLoading)(false);
    }
  };

  createEffect(
    on(
      () => {
        const a = args();
        return a ? JSON.stringify([hostId(), a]) : undefined;
      },
      (key) => {
        const a = args();
        if (!key || !a) {
          seq += 1;
          setHits([]);
          setTotal(0);
          setCursor(undefined);
          setLoading(false);
          setLoadingMore(false);
          setError(undefined);
          return;
        }
        setCursor(undefined);
        void fetchPage(a, undefined, false);
      },
    ),
  );
  onCleanup(() => {
    seq += 1;
  });

  const capped = () => hits().length >= SEARCH_MAX_ROWS;
  return {
    hits,
    total,
    loading,
    loadingMore,
    error,
    hasMore: () => !!cursor() && !capped(),
    capped,
    more: () => {
      const a = args();
      const at = cursor();
      if (!a || !at || loadingMore() || capped()) return;
      void fetchPage(a, at, true);
    },
    retry: () => {
      const a = args();
      if (a) void fetchPage(a, undefined, false);
    },
  };
}

/** Test support. */
export function resetSidebarSearch(): void {
  clearSidebarSearch();
}
