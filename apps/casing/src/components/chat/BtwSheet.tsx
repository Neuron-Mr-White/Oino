/**
 * Side question sheet (UNI-159): "+" menu, typed `/btw <question>` or
 * `/unipi:btw <question>` all open this. A question field, the streamed
 * markdown answer (same renderer as the main chat), earlier pages listed
 * below, and "Ask in main chat" which copies the Q&A into the composer
 * draft and closes. Works while pi is busy; never touches the main
 * transcript (the bridge runs it in a fresh read-only in-memory session).
 *
 * UNI-219: opening the sheet re-lists (`btw_list{watch}`) and the store
 * folds the list into this chat's own pages, so a page whose stream was
 * lost to a reconnect gets its answer in place (never a second copy). A
 * page says "Thinking…" only while really running, with the model's
 * streamed reasoning and tool lines when there are any.
 */
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { MessageCircleQuestion, Send } from 'lucide-solid';
import type { BtwListPage, Chat } from '../../lib/chat/store';
import { btwListRunning } from '../../lib/chat/store';
import { BoardSheet } from '../board/BoardSheet';
import { renderBlocks } from '../../lib/chat/markdown';
import { MdBlock } from './MdBlock';
import { copyText } from '../../lib/board/share';

export function BtwSheet(props: {
  open: boolean;
  onClose: () => void;
  chat: Chat;
  initialQuestion?: string;
  onAskInMainChat: (text: string) => void;
}) {
  const s = () => props.chat.state;
  const [question, setQuestion] = createSignal('');
  const [older, setOlder] = createSignal<BtwListPage[]>([]);
  const [loadingOlder, setLoadingOlder] = createSignal(false);

  createEffect(() => {
    if (!props.open) return;
    setQuestion(props.initialQuestion ?? '');
    setLoadingOlder(true);
    void props.chat.btwList().then((pages) => {
      // Newest first, like this chat's own pages.
      setOlder([...pages].reverse());
      setLoadingOlder(false);
    });
  });

  // While the sheet is open and an earlier page (TUI / another phone) is
  // still running, refresh the list so it doesn't sit on "Thinking…".
  createEffect(() => {
    if (!props.open || !older().some(btwListRunning)) return;
    const timer = setTimeout(() => void props.chat.btwList().then((pages) => setOlder([...pages].reverse())), 2500);
    onCleanup(() => clearTimeout(timer));
  });

  const ask = () => {
    const q = question().trim();
    if (!q) return;
    props.chat.askBtw(q);
    setQuestion('');
  };

  // Newest first: this chat's own streamed pages (reverse), then the older ones from btw_list.
  const ownPages = createMemo(() => [...s().btwPages].reverse());

  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Side question" testId="btw-sheet">
      <div class="btw-sheet-q">
        <textarea
          class="btw-sheet-field"
          rows={1}
          placeholder="Ask a quick side question…"
          value={question()}
          onInput={(e) => setQuestion(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault();
              ask();
            }
          }}
        />
        <button type="button" class="btw-sheet-ask" aria-label="Ask" disabled={!question().trim() || s().conn !== 'live'} onClick={ask}>
          <Send size={16} />
        </button>
      </div>
      <For each={ownPages()}>
        {(page) => (
          <div class="btw-page">
            <div class="btw-page-q">
              <MessageCircleQuestion size={15} />
              <span class="truncate">{page.question}</span>
            </div>
            <BtwProgress running={!page.done} answer={page.answer} thinking={page.thinking} toolLines={page.toolLines} />
            <Show when={page.error}>
              <p class="btw-page-error">{page.error}</p>
            </Show>
            <Show when={page.answer}>
              <div class="btw-page-answer md">
                <For each={renderBlocks(page.answer, !page.done)}>{(html) => <MdBlock html={html} />}</For>
              </div>
            </Show>
            <Show when={page.done && (page.answer || page.error)}>
              <div class="btw-page-foot">
                <button
                  type="button"
                  class="btw-copy-btn"
                  onClick={() => {
                    props.onAskInMainChat(`> ${page.question}\n> ${page.answer}\n`);
                    props.onClose();
                  }}
                >
                  Ask in main chat
                </button>
                <button type="button" class="btw-copy-btn" onClick={() => void copyText(page.answer)}>
                  Copy
                </button>
              </div>
            </Show>
          </div>
        )}
      </For>
      <Show when={!loadingOlder() && older().length > 0}>
        <For each={older()}>
          {(page) => (
            <div class="btw-page">
              <div class="btw-page-q">
                <MessageCircleQuestion size={15} />
                <span class="truncate">{page.question}</span>
              </div>
              <BtwProgress running={btwListRunning(page)} answer={page.answer} thinking={page.thinking} toolLines={page.toolLines} />
              <Show when={page.error}>
                <p class="btw-page-error">{page.error}</p>
              </Show>
              <Show when={page.answer}>
                <div class="btw-page-answer md">
                  <For each={renderBlocks(page.answer, btwListRunning(page))}>{(html) => <MdBlock html={html} />}</For>
                </div>
              </Show>
            </div>
          )}
        </For>
      </Show>
      <Show when={!loadingOlder() && !older().length && !ownPages().length}>
        <p class="btw-empty">No side questions yet. Ask something quick without touching the main chat.</p>
      </Show>
    </BoardSheet>
  );
}

/** Tool lines (always) and, while running, the "Thinking…" shimmer plus the
 *  tail of the model's streamed reasoning (until answer text arrives). */
function BtwProgress(props: { running: boolean; answer: string; thinking?: string; toolLines?: string[] }) {
  const thinkingTail = () => {
    const t = (props.thinking ?? '').trim();
    return t.length > 280 ? `…${t.slice(-280)}` : t;
  };
  return (
    <>
      <Show when={props.toolLines?.length}>
        <ul class="btw-page-tools" data-testid="btw-tools" style={{ margin: '0 0 6px', padding: '0', 'list-style': 'none', 'font-size': '12.5px', 'line-height': '18px', color: 'var(--color-text-muted)' }}>
          <For each={props.toolLines}>{(line) => <li class="truncate">{line}</li>}</For>
        </ul>
      </Show>
      <Show when={props.running && !props.answer}>
        <p class="btw-page-answer shimmer" data-testid="btw-thinking">Thinking…</p>
        <Show when={thinkingTail()}>
          <p class="btw-page-thinking" data-testid="btw-thinking-text" style={{ 'margin-top': '4px', 'font-size': '12.5px', 'line-height': '18px', 'font-style': 'italic', color: 'var(--color-text-muted)', 'white-space': 'pre-wrap', 'overflow-wrap': 'anywhere' }}>
            {thinkingTail()}
          </p>
        </Show>
      </Show>
    </>
  );
}
