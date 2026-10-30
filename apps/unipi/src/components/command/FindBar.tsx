/**
 * The find bar (WP-A18, Z-79): Cmd/Ctrl+F over the conversation. Loaded text
 * highlights in place inside the virtual timeline (matches in unmounted turns
 * scroll into view, then highlight); ↑/↓/Enter/Shift+Enter wrap; "Search
 * whole session" falls back to the host `search` command (SearchSheet logic,
 * one surface) with a results list under the bar. Rendered by `Timeline`
 * inside the conversation frame; Esc closes (unless already consumed).
 */
import { createEffect, createMemo, on, onCleanup, Show, For } from 'solid-js';
import { ArrowDown, ArrowUp, Search, X } from 'lucide-solid';
import {
  ROLE_LABEL,
  closeFind,
  findState,
  gotoFindMatch,
  jumpToHit,
  searchWholeSession,
  setFindQuery,
  stepFind,
  wholeSession,
  type SearchHit,
} from '../../lib/find';
import type { SessionHandle } from '../../lib/session/store';
import { agoText } from '../status/lhProgress';
import './command.css';

export function FindBar(props: { session: SessionHandle; index: () => { total: number; query: string }; turnOf: (entryId: string) => string | undefined; jump: (turnId: string) => void }) {
  const state = findState;
  const whole = wholeSession;
  let input: HTMLInputElement | undefined;
  createEffect(
    on(state, (s, prev) => {
      if (s.open && !prev?.open) queueMicrotask(() => input?.focus());
    }),
  );
  // Window-level Esc: only unhandled Escs close the bar (nested popups keep theirs).
  const onEsc = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || e.defaultPrevented || !state().open) return;
    e.preventDefault();
    closeFind();
  };
  window.addEventListener('keydown', onEsc);
  onCleanup(() => window.removeEventListener('keydown', onEsc));
  const count = () => {
    const total = props.index().total;
    return total ? `${state().active + 1}/${total}` : '0';
  };
  const none = () => state().query.trim() && !props.index().total;

  const onKey = (event: KeyboardEvent) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      closeFind();
      return;
    }
    if (event.key === 'Enter') {
      event.preventDefault();
      stepFind(props.index().total, event.shiftKey ? -1 : 1);
      return;
    }
    if (event.key === 'ArrowDown' && event.altKey) {
      event.preventDefault();
      stepFind(props.index().total, 1);
    } else if (event.key === 'ArrowUp' && event.altKey) {
      event.preventDefault();
      stepFind(props.index().total, -1);
    }
  };

  return (
    <Show when={state().open}>
      <div class="cfind" role="search" aria-label="Find in chat" data-testid="find-bar">
        <div class="cfind-bar">
          <Search size={15} class="ml-1 shrink-0" style={{ color: 'var(--color-text-dim)' }} aria-hidden="true" />
          <span class="cfind-count" data-none={none() ? '' : undefined} data-testid="find-count" aria-live="polite">
            {count()}
          </span>
          <input
            ref={input}
            class="cfind-input"
            placeholder="Find in this chat…"
            aria-label="Find in this chat"
            value={state().query}
            onInput={(e) => setFindQuery(e.currentTarget.value)}
            onKeyDown={onKey}
            autocomplete="off"
            spellcheck={false}
          />
          <button type="button" class="cfind-btn" aria-label="Previous match (Shift+Enter)" disabled={!props.index().total} onClick={() => stepFind(props.index().total, -1)}>
            <ArrowUp size={15} />
          </button>
          <button type="button" class="cfind-btn" aria-label="Next match (Enter)" disabled={!props.index().total} onClick={() => stepFind(props.index().total, 1)}>
            <ArrowDown size={15} />
          </button>
          <button type="button" class="cfind-whole" title="Search the whole session on the host (also covers older branches)" disabled={!state().query.trim()} onClick={() => void searchWholeSession(props.session, state().query)}>
            Search whole session
          </button>
          <button type="button" class="cfind-btn" aria-label="Close find" onClick={() => closeFind()}>
            <X size={15} />
          </button>
        </div>
        <WholeResults whole={whole()} session={props.session} query={() => state().query} turnOf={props.turnOf} jump={props.jump} />
      </div>
    </Show>
  );
}

function WholeResults(props: { whole: ReturnType<typeof wholeSession>; session: SessionHandle; query: () => string; turnOf: (entryId: string) => string | undefined; jump: (turnId: string) => void }) {
  createEffect(() => {
    if (!props.whole.busy) return;
    queueMicrotask(() => document.querySelector('.cfind-results')?.scrollTo?.(0, 0));
  });
  return (
    <Show when={props.whole.busy || props.whole.hits !== null || props.whole.unsupported}>
      <div class="cfind-results" data-testid="find-whole">
        <Show
          when={!props.whole.unsupported}
          fallback={<p class="cfind-results-hint">Search needs a newer unipi on the PC.</p>}
        >
          <Show when={props.whole.hits} fallback={<p class="cfind-results-hint">Searching the whole session…</p>}>
            {(hits) => (
              <For each={hits()}>
                {(hit) => (
                  <button type="button" class="cfind-hit" onClick={() => void pick(hit)}>
                    <span class="cfind-hit-head">
                      <span class="cfind-role">{ROLE_LABEL[hit.role] ?? hit.role}</span>
                      <Show when={hit.timestamp}>
                        <span class="cfind-when">{agoText(Date.now() - Date.parse(hit.timestamp!))}</span>
                      </Show>
                    </span>
                    <span class="cfind-snippet">
                      <Marked text={hit.snippet} marks={hit.marks} />
                    </span>
                  </button>
                )}
              </For>
            )}
          </Show>
        </Show>
      </div>
    </Show>
  );

  function pick(hit: SearchHit) {
    gotoFindMatch(0);
    void jumpToHit(props.session, hit, props.turnOf, props.jump);
  }
}

/** The snippet with its matched words in `<mark>` (SearchSheet's Marked). */
export function Marked(props: { text: string; marks: Array<[number, number]> }) {
  const parts = createMemo(() => {
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
  });
  return (
    <For each={parts()}>
      {(p) => (p.hit ? <mark>{p.text}</mark> : <>{p.text}</>)}
    </For>
  );
}

