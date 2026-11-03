/**
 * Settings › Memory › Browse memories (owner fix 2026-11-03): the app's own
 * modal (title, X, Esc; the full-screen sheet on the phone) over the host's
 * `memory_browse` op — a search box, a project scope and the newest memories
 * 50 at a time. Search runs on the host; the webview never holds the store
 * (the owner's has ~7,000 memories), only the pages scrolled to (capped).
 *
 * It replaces running `/memory` in Settings' hidden command chat, whose
 * scratch folder made the project scope a random id (always empty), and whose
 * "All projects" shipped every memory in one UI patch (dropped as oversize).
 * Forgetting stays the extension's: "Open in chat" runs `/memory` in a chat.
 *
 *   openMemoryBrowser({ hostId, project })   // project: undefined = All projects
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { ArrowLeft, Brain, RotateCw, Search } from 'lucide-solid';
import { MemoryBrowseDataSchema, memoryProjectName, type MemoryItem, type MemoryProject } from '@unipi/protocol';
import { hostCall } from '../../lib/host/factory';
import { hubs } from '../../lib/session/hub';
import { isHomeCwd } from '../../lib/session/selectors';
import { manageChat } from '../../lib/settings/manage';
import { ModelDialog } from '../model/ModelPopover';
import { agoText } from '../status/lhProgress';
import { Markdown } from '../board/Markdown';
import './memory.css';

/** Memories per page (the host caps a page at 100). */
export const MEMORY_PAGE = 50;
/** The most rows the modal keeps loaded (Load more stops there; search narrows instead). */
export const MEMORY_ROWS_MAX = 1_000;
const SEARCH_DEBOUNCE_MS = 250;

export type MemoryBrowserTarget = { hostId: string; project?: string };
const [target, setTarget] = createSignal<MemoryBrowserTarget | undefined>();

export function openMemoryBrowser(next: MemoryBrowserTarget): void {
  setTarget(next);
}
export function closeMemoryBrowser(): void {
  setTarget(undefined);
}
export const memoryBrowserTarget = target;

/**
 * Settings / the command center: "Browse memories" scopes to the project of the
 * chat the user was last in (a real folder, never home or a scratch chat), else
 * every project; "All projects" always opens unscoped.
 */
export function openMemoryBrowserFor(scope: 'project' | 'all', hostId: string): void {
  let project: string | undefined;
  if (scope === 'project') {
    const chat = manageChat();
    const cwd = chat ? hubs().peek(chat.hostId)?.state.cards[chat.sessionId]?.cwd : undefined;
    if (cwd && !isHomeCwd(cwd)) project = memoryProjectName(cwd);
  }
  openMemoryBrowser({ hostId, ...(project ? { project } : {}) });
}

type PageState = {
  items: MemoryItem[];
  total: number;
  cursor?: string;
  /** First page in flight (skeleton). */
  loading: boolean;
  /** Next page in flight. */
  more: boolean;
  error?: string;
  store: boolean;
};

const EMPTY: PageState = { items: [], total: 0, loading: true, more: false, store: true };

/** Mounted once by the shell. */
export function MemoryBrowserLayer() {
  return (
    <Show when={target()} keyed>
      {(t) => <MemoryBrowser target={t} onClose={closeMemoryBrowser} />}
    </Show>
  );
}

export function MemoryBrowser(props: { target: MemoryBrowserTarget; onClose: () => void }) {
  const [query, setQuery] = createSignal('');
  const [debounced, setDebounced] = createSignal('');
  const [project, setProject] = createSignal<string | undefined>(props.target.project);
  const [projects, setProjects] = createSignal<MemoryProject[]>([]);
  const [page, setPage] = createSignal<PageState>(EMPTY);
  const [open, setOpen] = createSignal<MemoryItem | undefined>();
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => timer && clearTimeout(timer));

  createEffect(
    on(query, (q) => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => setDebounced(q.trim()), q.trim() ? SEARCH_DEBOUNCE_MS : 0);
    }),
  );

  const fetchPage = async (more: boolean) => {
    const mine = ++seq;
    const prev = page();
    if (more && (!prev.cursor || prev.more || prev.items.length >= MEMORY_ROWS_MAX)) return;
    setPage(more ? { ...prev, more: true, error: undefined } : { ...EMPTY, loading: true });
    const res = await hostCall(
      props.target.hostId,
      'memory_browse',
      {
        ...(project() ? { project: project() } : {}),
        ...(debounced() ? { query: debounced() } : {}),
        limit: MEMORY_PAGE,
        ...(more && prev.cursor ? { cursor: prev.cursor } : {}),
      },
      MemoryBrowseDataSchema,
    );
    if (mine !== seq) return; // a newer search / scope won
    if (!res.ok) {
      const msg = res.error.code === 'E_UNSUPPORTED' || res.error.code === 'E_SCHEMA' ? 'This PC’s host is too old to browse memories here — update it, or use /memory in a chat.' : res.error.message || res.error.code;
      setPage({ ...(more ? prev : EMPTY), loading: false, more: false, error: msg });
      return;
    }
    const data = res.value;
    if (!more && data.projects.length) setProjects(data.projects);
    const items = more ? [...prev.items, ...data.items].slice(0, MEMORY_ROWS_MAX) : data.items;
    setPage({ items, total: data.total, cursor: data.next_cursor, loading: false, more: false, store: data.store });
  };

  // Scope or the (debounced) search changed: the first page again.
  createEffect(on([project, debounced], () => void fetchPage(false)));

  const scopeLabel = () => (project() ? project()! : 'All projects');
  const countLine = createMemo(() => {
    const p = page();
    if (p.loading) return '';
    const n = p.total.toLocaleString();
    const what = p.total === 1 ? 'memory' : 'memories';
    return debounced() ? `${n} ${p.total === 1 ? 'match' : 'matches'} in ${scopeLabel()}` : `${n} ${what} in ${scopeLabel()} · newest first`;
  });
  const canMore = () => !!page().cursor && page().items.length < MEMORY_ROWS_MAX;
  const onScroll = (e: Event) => {
    const el = e.currentTarget as HTMLElement;
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - 120 && canMore() && !page().more && !page().error) void fetchPage(true);
  };

  return (
    <ModelDialog title="Memories" testId="memory-browser" onClose={() => (open() ? setOpen(undefined) : props.onClose())}>
      <Show
        when={open()}
        fallback={
          <div class="mb-root">
            <div class="mb-tools">
              <label class="mb-search">
                <Search size={15} aria-hidden="true" />
                <input
                  type="search"
                  placeholder="Search memories…"
                  aria-label="Search memories"
                  value={query()}
                  onInput={(e) => setQuery(e.currentTarget.value)}
                  data-testid="memory-search"
                  autofocus
                />
                <Show when={page().loading && !!debounced()}>
                  <span class="mp-spinner" aria-label="Searching" />
                </Show>
              </label>
              <select class="mb-scope" aria-label="Project" value={project() ?? ''} onChange={(e) => setProject(e.currentTarget.value || undefined)} data-testid="memory-scope">
                <option value="">All projects</option>
                <Show when={project() && !projects().some((p) => p.name === project())}>
                  <option value={project()}>{project()}</option>
                </Show>
                <For each={projects()}>{(p) => <option value={p.name}>{`${p.name} (${p.count.toLocaleString()})`}</option>}</For>
              </select>
            </div>
            <p class="mb-count" data-testid="memory-count" aria-live="polite">
              {countLine()}
            </p>
            <div class="mb-list" role="list" onScroll={onScroll} data-testid="memory-list">
              <Show when={!page().loading} fallback={<Skeleton />}>
                <Show
                  when={page().items.length}
                  fallback={
                    <Show when={!page().error}>
                      <div class="mb-empty" data-testid="memory-empty">
                        <Brain size={20} aria-hidden="true" />
                        <p>
                          {debounced()
                            ? `No memory in ${scopeLabel()} matches “${debounced()}”.`
                            : !page().store
                              ? 'Nothing remembered on this PC yet. The agent saves memories as it works.'
                              : `Nothing remembered in ${scopeLabel()} yet.`}
                        </p>
                      </div>
                    </Show>
                  }
                >
                  <For each={page().items}>
                    {(m) => (
                      <button type="button" role="listitem" class="mb-row" onClick={() => setOpen(m)} data-testid="memory-row">
                        <span class="mb-row-title">{m.title}</span>
                        <span class="mb-row-sub">
                          {m.kind} · {m.project} · {agoText(Date.now() - m.updated_at)}
                        </span>
                      </button>
                    )}
                  </For>
                  <Show when={page().more}>
                    <div class="mb-more-note"><span class="mp-spinner" aria-hidden="true" /> Loading more…</div>
                  </Show>
                  <Show when={canMore() && !page().more && !page().error}>
                    <button type="button" class="mb-more" onClick={() => void fetchPage(true)} data-testid="memory-more">
                      Load {Math.min(MEMORY_PAGE, page().total - page().items.length).toLocaleString()} more
                    </button>
                  </Show>
                  <Show when={!page().cursor || page().items.length >= MEMORY_ROWS_MAX}>
                    <Show when={page().items.length < page().total}>
                      <p class="mb-more-note">Showing the newest {page().items.length.toLocaleString()} — search to find older ones.</p>
                    </Show>
                  </Show>
                </Show>
              </Show>
              <Show when={page().error}>
                {(err) => (
                  <div class="mb-error" role="alert" data-testid="memory-error">
                    <p>Couldn’t load memories: {err()}</p>
                    <button type="button" class="btn btn-secondary" onClick={() => void fetchPage(page().items.length > 0)} data-testid="memory-retry">
                      <RotateCw size={14} /> Retry
                    </button>
                  </div>
                )}
              </Show>
            </div>
          </div>
        }
      >
        {(m) => (
          <div class="mb-detail" data-testid="memory-detail">
            <button type="button" class="mb-back" onClick={() => setOpen(undefined)} data-testid="memory-back">
              <ArrowLeft size={15} /> All memories
            </button>
            <h3 class="mb-detail-title">{m().title}</h3>
            <p class="mb-row-sub">
              {m().kind} · {m().project} · {agoText(Date.now() - m().updated_at)}
            </p>
            <div class="mb-detail-body">
              <Markdown text={m().body + (m().truncated ? '\n\n…' : '')} class="chat-prose" />
            </div>
          </div>
        )}
      </Show>
    </ModelDialog>
  );
}

function Skeleton() {
  return (
    <div class="mb-skeleton" aria-label="Loading memories" data-testid="memory-skeleton">
      <For each={[0, 1, 2, 3, 4, 5]}>
        {() => (
          <div class="mb-skeleton-row">
            <span />
            <span />
          </div>
        )}
      </For>
    </div>
  );
}
