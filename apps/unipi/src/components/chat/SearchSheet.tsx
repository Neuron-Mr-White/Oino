/**
 * UNI-275: search this chat. The query goes to pi (`search`): the WHOLE
 * session is searched, not only what the phone has paged in, and results
 * are ranked by similarity (word stems, typos, phrase, who said it) rather
 * than exact text. Tapping a result navigates there: older history pages in
 * up to it (the whole chat stays loaded) and the message flashes.
 */
import { createEffect, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { Search as SearchIcon, X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { EmptyState } from '../EmptyState';
import type { Chat, SearchHit } from '../../lib/chat/store';
import { agoText } from '../../lib/chat/progress';

const ROLE_LABEL: Record<string, string> = { user: 'You', assistant: 'pi', tool: 'Ran', output: 'Output', command: 'Command', note: 'Note' };

/** The snippet with its matched words in <mark>. */
export function Marked(props: { text: string; marks: Array<[number, number]> }) {
  const parts = () => {
    const out: Array<{ text: string; hit: boolean }> = [];
    let at = 0;
    for (const [s, e] of [...props.marks].sort((a, b) => a[0] - b[0])) {
      if (s < at) continue;
      if (s > at) out.push({ text: props.text.slice(at, s), hit: false });
      out.push({ text: props.text.slice(s, e), hit: true });
      at = e;
    }
    if (at < props.text.length) out.push({ text: props.text.slice(at), hit: false });
    return out;
  };
  return <For each={parts()}>{(p) => (p.hit ? <mark>{p.text}</mark> : <>{p.text}</>)}</For>;
}

export function SearchSheet(props: { open: boolean; onClose: () => void; chat: Chat; onPick: (hit: SearchHit) => void }) {
  const [query, setQuery] = createSignal('');
  const [hits, setHits] = createSignal<SearchHit[] | null | undefined>(undefined);
  const [busy, setBusy] = createSignal(false);
  let input: HTMLInputElement | undefined;
  let seq = 0;
  createEffect(
    on(
      () => props.open,
      (open) => {
        if (open) queueMicrotask(() => input?.focus());
      },
    ),
  );
  createEffect(
    on(query, (q) => {
      const n = ++seq;
      if (!q.trim()) {
        setHits(undefined);
        setBusy(false);
        return;
      }
      setBusy(true);
      const t = setTimeout(() => {
        void props.chat.search(q.trim()).then((res) => {
          if (n !== seq) return;
          setHits(res);
          setBusy(false);
        });
      }, 220);
      onCleanup(() => clearTimeout(t));
    }),
  );
  const now = () => Date.now();
  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Search in chat" class="search-sheet" testId="search-sheet">
      <div class="search-field">
        <SearchIcon size={16} class="search-field-icon" />
        <input
          ref={input}
          type="search"
          class="search-input"
          placeholder="Search this chat…"
          aria-label="Search this chat"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && hits()?.[0]) props.onPick(hits()![0]!);
          }}
        />
        <Show when={query()}>
          <button type="button" class="search-clear" aria-label="Clear" onClick={() => (setQuery(''), input?.focus())}>
            <X size={14} />
          </button>
        </Show>
      </div>
      <Show
        when={hits()}
        fallback={
          <Show when={hits() === null} fallback={<p class="search-hint">{busy() ? 'Searching…' : 'Finds similar words too — typos, other word forms, phrases.'}</p>}>
            <p class="search-hint">Search needs a newer unipi on the PC.</p>
          </Show>
        }
      >
        {(list) => (
          <Show when={list().length > 0} fallback={<EmptyState compact testId="search-empty" icon={<SearchIcon size={22} />} title="Nothing found" body="Try fewer or different words." />}>
            <p class="search-count" aria-live="polite">
              {list().length === 60 ? '60+' : list().length} {list().length === 1 ? 'result' : 'results'}
              {busy() ? ' · searching…' : ''}
            </p>
            <ul class="search-list" data-testid="search-results">
              <For each={list()}>
                {(hit) => (
                  <li>
                    <button type="button" class="search-row" data-role={hit.role} onClick={() => props.onPick(hit)}>
                      <span class="search-row-head">
                        <span class="search-role">{ROLE_LABEL[hit.role] ?? hit.role}</span>
                        <Show when={hit.timestamp}>
                          <span class="search-when">{agoText(now() - Date.parse(hit.timestamp!))}</span>
                        </Show>
                      </span>
                      <span class="search-snippet">
                        <Marked text={hit.snippet} marks={hit.marks} />
                      </span>
                    </button>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        )}
      </Show>
    </BoardSheet>
  );
}
