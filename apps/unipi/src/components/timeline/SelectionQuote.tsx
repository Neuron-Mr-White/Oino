/**
 * Text selection → Quote (app spec WP-A7, Z-30): on the desktop shell,
 * selecting text inside the timeline shows a small floating "Quote" button
 * above the selection; it puts the selection in the composer as a quote chip
 * and clears the selection. The phone keeps swipe-to-quote instead
 * (`attachQuoteSwipe`), so nothing floats over the native selection handles.
 *
 * The menu is a portal (fixed position), clamps to the viewport, never steals
 * the selection (`mousedown` is prevented), and disappears when the selection
 * collapses, the timeline scrolls, or Esc is pressed.
 */
import { createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { Quote } from 'lucide-solid';
import { desktopShell } from '../../lib/layout';
import { quoteIntoComposer } from '../../lib/timeline/actions';

type Picked = { text: string; left: number; top: number };

const MAX_SELECTION = 4_000;

export function SelectionQuote(props: { root: () => HTMLElement | undefined; sessionKey: () => string }) {
  const [picked, setPicked] = createSignal<Picked | undefined>();

  const read = () => {
    if (!desktopShell()) return;
    const root = props.root();
    const selection = window.getSelection();
    if (!root || !selection || selection.isCollapsed || !selection.rangeCount) {
      setPicked(undefined);
      return;
    }
    const range = selection.getRangeAt(0);
    if (!root.contains(range.commonAncestorContainer)) {
      setPicked(undefined);
      return;
    }
    const text = selection.toString().trim();
    if (!text) {
      setPicked(undefined);
      return;
    }
    const rect = range.getBoundingClientRect();
    const width = Math.min(MAX_SELECTION, text.length);
    const left = Math.max(12, Math.min(window.innerWidth - 96, rect.left + rect.width / 2 - 32));
    const top = rect.top > 56 ? rect.top - 40 : Math.min(window.innerHeight - 48, rect.bottom + 8);
    setPicked({ text: text.slice(0, width), left, top });
  };

  onMount(() => {
    let frame = 0;
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(read);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setPicked(undefined);
    };
    document.addEventListener('selectionchange', schedule);
    document.addEventListener('keydown', onKey);
    const root = props.root();
    const onScroll = () => setPicked(undefined);
    root?.addEventListener('scroll', onScroll, { passive: true });
    onCleanup(() => {
      cancelAnimationFrame(frame);
      document.removeEventListener('selectionchange', schedule);
      document.removeEventListener('keydown', onKey);
      root?.removeEventListener('scroll', onScroll);
    });
  });

  const quote = () => {
    const value = picked();
    if (!value) return;
    quoteIntoComposer(props.sessionKey(), value.text);
    window.getSelection()?.removeAllRanges();
    setPicked(undefined);
  };

  return (
    <Show when={picked()} keyed>
      {(value) => (
        <Portal>
          <div
            class="tl-selection-quote"
            data-testid="tl-selection-quote"
            style={{ left: `${value.left}px`, top: `${value.top}px` }}
            onMouseDown={(event) => event.preventDefault()}
            onPointerDown={(event) => event.preventDefault()}
          >
            <button type="button" onMouseDown={(event) => event.preventDefault()} onClick={quote}>
              <Quote size={13} /> Quote
            </button>
          </div>
        </Portal>
      )}
    </Show>
  );
}
