/**
 * File search (WP-A20, Z-101): fuzzy **name** search (client-ranked over
 * `fs_search kind:name` results) and **content** search (`fs_search
 * kind:content`, line numbers + preview), over the project root. Picking a
 * result opens it: text files in the editor area (jumped to the line),
 * media in the preview.
 *
 * Search runs as you type (150 ms debounce) — there is no "Search" button to
 * press twice; Enter opens the first hit. The panel sits in the tree column,
 * so its header is two rows (query, then the name/content scope) and every
 * hit keeps its path on one ellipsised line.
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { FileText, Search, AlignLeft, X } from 'lucide-solid';
import { basename, dirname, fsSearch } from '../../lib/fs';
import { rankByName } from '../../lib/files/tree';
import { pushBack } from '../../lib/back';

export type SearchKind = 'name' | 'content';
type NameResult = { path: string };
type ContentResult = { path: string; line?: number; preview?: string };

const DEBOUNCE_MS = 150;

export function FileSearch(props: {
  hostId: string;
  root: () => string | undefined;
  onOpen: (path: string, line?: number) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = createSignal('');
  const [kind, setKind] = createSignal<SearchKind>('name');
  const [results, setResults] = createSignal<{ name: NameResult[]; content: ContentResult[] }>({ name: [], content: [] });
  const [more, setMore] = createSignal(false);
  const [state, setState] = createSignal<'idle' | 'running' | 'done' | 'error'>('idle');
  const [error, setError] = createSignal<string>();
  let token = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => onCleanup(pushBack(props.onClose)));
  onCleanup(() => clearTimeout(timer));

  async function run() {
    const q = query().trim();
    const root = props.root();
    const mine = ++token;
    if (!q || !root) {
      setState('idle');
      setResults({ name: [], content: [] });
      return;
    }
    setState('running');
    setError(undefined);
    const name = await fsSearch(props.hostId, root, q, 'name');
    if (mine !== token) return;
    if (!name.ok) {
      setState('error');
      setError(name.error.message);
      return;
    }
    const content = q.length >= 2 ? await fsSearch(props.hostId, root, q, 'content') : { ok: true as const, value: { results: [], more: false } };
    if (mine !== token) return;
    const contentResults = content.ok ? content.value.results : [];
    setResults({
      name: rankByName(q, name.value.results.map((r) => r.path)).map((hit) => ({ path: hit.path })),
      content: contentResults,
    });
    setMore(name.value.more || (content.ok && content.value.more));
    setState('done');
  }

  // Live search: the query and the scope both re-run (debounced).
  createEffect(() => {
    const q = query();
    kind();
    clearTimeout(timer);
    if (!q.trim()) return;
    timer = setTimeout(() => void run(), DEBOUNCE_MS);
  });

  const hits = () => results()[kind()] as Array<{ path: string; line?: number; preview?: string }>;

  return (
    <div class="fsearch" data-testid="files-search-panel" role="dialog" aria-label="Search files">
      <div class="fsearch-bar">
        <Search size={15} class="fsearch-icon" />
        <input
          data-testid="files-search-input"
          placeholder="Search this project…"
          value={query()}
          autofocus
          aria-label="Search files"
          onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              const first = hits()[0];
              if (first) pick(first);
              else void run();
            }
            if (e.key === 'Escape') props.onClose();
          }}
        />
        <button type="button" class="ftree-tool" aria-label="Close search" data-testid="files-search-close" onClick={props.onClose}>
          <X size={15} />
        </button>
      </div>
      <div class="fsearch-scope">
        <div class="fsearch-kinds" role="tablist" aria-label="Search in">
          <button type="button" role="tab" aria-selected={kind() === 'name'} data-testid="files-search-name" onClick={() => setKind('name')}>
            <FileText size={13} /> Name
          </button>
          <button type="button" role="tab" aria-selected={kind() === 'content'} data-testid="files-search-content" onClick={() => setKind('content')}>
            <AlignLeft size={13} /> Content
          </button>
        </div>
        <span class="fsearch-count" role="status" data-testid="files-search-count" title={state() === 'done' ? `${hits().length} match${hits().length === 1 ? '' : 'es'}` : undefined}>
          <Show when={state() === 'running'}>…</Show>
          <Show when={state() === 'done' && hits().length > 0}>{hits().length}</Show>
        </span>
      </div>
      <div class="fsearch-results" data-testid="files-search-results">
        <Show when={state() === 'error'}>
          <p class="fsearch-note" role="alert">{error()}</p>
        </Show>
        <Show when={state() === 'idle' && !query()}>
          <p class="fsearch-note">Find files by name, or text inside them (≥ 2 characters).</p>
        </Show>
        <Show when={state() === 'done' && hits().length === 0}>
          <p class="fsearch-note" data-testid="files-search-empty">No matches for “{query()}”.</p>
        </Show>
        <For each={hits()}>
          {(hit) => (
            <button type="button" class="fsearch-row" data-testid="files-search-row" data-path={hit.path} title={hit.path} onClick={() => pick(hit)}>
              <span class="fsearch-row-head">
                <span class="fsearch-row-name">{basename(hit.path)}</span>
                <Show when={hit.line}>
                  <span class="fsearch-row-line">:{hit.line}</span>
                </Show>
              </span>
              <span class="fsearch-row-path">{hit.line ? dirname(hit.path) : hit.path}</span>
              <Show when={hit.preview}>
                <span class="fsearch-row-preview">{hit.preview}</span>
              </Show>
            </button>
          )}
        </For>
        <Show when={more()}>
          <p class="fsearch-note">Showing the first {hits().length} matches — refine the query to narrow it down.</p>
        </Show>
      </div>
    </div>
  );

  function pick(hit: { path: string; line?: number }) {
    props.onOpen(hit.path, hit.line);
  }
}
