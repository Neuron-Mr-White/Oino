/**
 * The `btw` side-pane tab (WP-A10, Z-31): the multi-turn side chat with btw
 * semantics — read-only, it **never touches the main session**. Questions go
 * through `_oino/service_call` (`unipi.btw` serves `btw.ask` / `btw.list`);
 * earlier pages (this tab, the TUI, another phone) list below, newest first,
 * and "Ask in main chat" copies a Q&A into the composer draft.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { MessageCircleQuestion, Send } from 'lucide-solid';
import { copyText } from '../../../lib/board/share';
import { renderBlocks } from '../../../lib/chat/markdown';
import { setComposerText, focusComposer } from '../../../lib/composer/drafts';
import { sessions, type SessionHandle } from '../../../lib/session/store';
import { setPaneComponent, type PaneTabProps } from '../../../lib/panes/kinds';
import { MdBlock } from '../../chat/MdBlock';
import { tabBaseOf } from './sessionTabs';
import './tabs.css';

/** One `btw.list` page (the extension's `list_page` shape). */
export type BtwPage = {
  id: string;
  question: string;
  answer: string;
  done: boolean;
  running: boolean;
  error?: string;
  toolLines?: string[];
  thinking?: string;
};

const listPages = (s: SessionHandle): Promise<BtwPage[]> =>
  s
    .cmd<unknown>({ type: '_oino/service_call', target: 'unipi.btw', name: 'btw.list', payload: {} })
    .then((reply) => (Array.isArray(reply) ? (reply as BtwPage[]).filter((p) => p && typeof p.id === 'string') : []));

const askBtw = (s: SessionHandle, question: string) =>
  s.cmd<{ id?: string; answer?: string; error?: string }>({ type: '_oino/service_call', target: 'unipi.btw', name: 'btw.ask', payload: { question } });

export function BtwTab(props: PaneTabProps) {
  const base = () => tabBaseOf(props.tab.params, { hostId: props.hostId, sessionId: props.sessionId });
  const handle = () => (base().sessionId ? sessions().ensure(base().hostId, base().sessionId) : undefined);
  const [pages, setPages] = createSignal<BtwPage[]>([]);
  const [question, setQuestion] = createSignal('');
  const [asking, setAsking] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();

  const refresh = () => {
    const s = handle();
    if (!s) return Promise.resolve();
    return listPages(s).then((list) => setPages([...list].reverse()), () => undefined);
  };
  // Live only: `cmd` rejects while the stream is still connecting.
  const live = () => (handle()?.state.conn === 'live' ? handle() : undefined);
  createEffect(() => {
    if (live()) void refresh();
  });

  // While a page runs (this tab, the TUI, another phone), keep the list fresh.
  createEffect(() => {
    if (!pages().some((p) => p.running)) return;
    const timer = setTimeout(() => void refresh(), 2500);
    onCleanup(() => clearTimeout(timer));
  });

  const ask = async (text?: string) => {
    const s = live();
    const q = (text ?? question()).trim();
    if (!s || !q || asking()) return;
    setAsking(true);
    setError(undefined);
    setQuestion('');
    try {
      const reply = await askBtw(s, q);
      if (reply?.error) setError(reply.error);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setAsking(false);
      await refresh();
    }
  };

  // `/btw <question>` seeds the tab with its first question (consumed once,
  // and only once the stream is live so the ask is not lost to a connecting
  // session).
  createEffect(() => {
    const q = props.tab.params?.question;
    const s = live();
    if (typeof q !== 'string' || !q.trim() || !s || askedSeeds.has(props.tab.id)) return;
    askedSeeds.add(props.tab.id);
    setQuestion('');
    void ask(q);
  });

  const own = createMemo<BtwPage[]>(() => (asking() ? [{ id: 'asking', question: pendingQ, answer: '', done: false, running: true }, ...pages()] : pages()));
  let pendingQ = '';

  const send = () => {
    pendingQ = question().trim();
    void ask();
  };

  const intoComposer = (page: BtwPage) => {
    const s = handle();
    if (!s) return;
    setComposerText(s.key, `> ${page.question}\n> ${page.answer}\n`);
    focusComposer(s.key);
  };

  return (
    <div class="btab" data-testid="btw-tab">
      <div class="btab-list">
        <For each={own()}>
          {(page) => (
            <div class="btab-page" data-testid="btw-page">
              <div class="btab-q">
                <MessageCircleQuestion size={15} class="shrink-0" />
                <span class="truncate">{page.question}</span>
              </div>
              <Show when={page.toolLines?.length}>
                <ul class="btab-tools" data-testid="btw-tools">
                  <For each={page.toolLines}>{(line) => <li class="truncate">{line}</li>}</For>
                </ul>
              </Show>
              <Show when={page.running && !page.answer}>
                <p class="btab-thinking shimmer" data-testid="btw-thinking">
                  Thinking…
                </p>
                <Show when={(page.thinking ?? '').trim()}>
                  <p class="btab-think">{thinkingTail(page.thinking!)}</p>
                </Show>
              </Show>
              <Show when={page.error}>
                <p class="btab-error">{page.error}</p>
              </Show>
              <Show when={page.answer}>
                <div class="btab-answer md" data-testid="btw-answer">
                  <For each={renderBlocks(page.answer, page.running)}>{(html) => <MdBlock html={html} />}</For>
                </div>
              </Show>
              <Show when={page.done && (page.answer || page.error)}>
                <div class="btab-foot">
                  <button type="button" class="btab-action" onClick={() => intoComposer(page)} data-testid="btw-ask-main">
                    Ask in main chat
                  </button>
                  <Show when={page.answer}>
                    <button type="button" class="btab-action" onClick={() => void copyText(page.answer)}>
                      Copy
                    </button>
                  </Show>
                </div>
              </Show>
            </div>
          )}
        </For>
        <Show when={error()}>
          <p class="btab-error" data-testid="btw-error">
            {error()}
          </p>
        </Show>
        <Show when={!asking() && own().length === 0}>
          <p class="btab-empty">No side questions yet. Ask something quick without touching the main chat.</p>
        </Show>
      </div>
      <div class="btab-ask">
        <textarea
          class="btab-field"
          rows={1}
          placeholder="Ask a quick side question…"
          value={question()}
          onInput={(e) => setQuestion(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
        />
        <button type="button" class="btab-send" aria-label="Ask" disabled={!question().trim() || asking()} onClick={send} data-testid="btw-ask">
          <Send size={16} />
        </button>
      </div>
    </div>
  );
}

function thinkingTail(text: string): string {
  const t = text.trim();
  return t.length > 280 ? `…${t.slice(-280)}` : t;
}

/** Tabs whose seeded `/btw` question was already asked (a persisted param asks once). */
const askedSeeds = new Set<string>();

setPaneComponent('btw', BtwTab);
