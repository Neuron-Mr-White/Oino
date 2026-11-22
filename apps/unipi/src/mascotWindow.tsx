/**
 * UNI-200: the desktop mascot window — a small fully transparent, frameless,
 * always-on-top Tauri window (UNI-255: only the crab, no tile; 72/96/128 px) (label `mascot`, mascot.html) with the Unicrab
 * and its reply bubble. The main window feeds it (`mascot-feed`, see
 * components/MascotFeedRunner.tsx); clicking the crab focuses UniPi,
 * clicking a bubble opens that chat (`mascot-open`), dragging moves the
 * window. Off by default (Settings → Unicrab, or the tray).
 */
import { render } from 'solid-js/web';
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Unicrab } from './components/Unicrab';
import { BUBBLE_MS, DEFAULT_MASCOT_WINDOW_SIZE, type MascotFeed } from './lib/mascot';
import type { MascotState } from './lib/unicrab';
import './style.css';
import './components/needsYou.css';
import './components/mascot.css';

export function MascotWindow(props: { emit: (event: string, payload?: unknown) => void; startDrag: () => void }) {
  const [state, setState] = createSignal<MascotState>('idle');
  const [count, setCount] = createSignal(0);
  const [bubble, setBubble] = createSignal<NonNullable<MascotFeed['bubble']> | null>(null);
  // UNI-255: just the crab (no tile behind it), at the chosen size.
  const [size, setSize] = createSignal<number>(DEFAULT_MASCOT_WINDOW_SIZE);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(timer));

  const apply = (feed: MascotFeed) => {
    setState(feed.state);
    setCount(feed.count);
    if (feed.size && feed.size >= 32 && feed.size <= 256) setSize(feed.size);
    if (feed.bubble && feed.bubble.id !== bubble()?.id) {
      setBubble(feed.bubble);
      clearTimeout(timer);
      timer = setTimeout(() => setBubble(null), BUBBLE_MS);
    }
  };
  (window as unknown as { __unipiMascotFeed?: (f: MascotFeed) => void }).__unipiMascotFeed = apply;

  let downAt: { x: number; y: number } | null = null;
  return (
    <div class="mascot-window" data-testid="mascot-window">
      <For each={bubble() ? [bubble()!] : []}>
        {(b) => (
          <button type="button" class="mascot-bubble" data-testid="mascot-bubble" onClick={() => (setBubble(null), props.emit('mascot-open', { link: b.link }))}>
            <span class="mascot-bubble-text">{b.text}</span>
          </button>
        )}
      </For>
      <button
        type="button"
        class="mascot-button mascot-crab-only"
        aria-label="Open Unipi"
        onMouseDown={(e) => {
          downAt = { x: e.clientX, y: e.clientY };
        }}
        onMouseMove={(e) => {
          if (downAt && Math.hypot(e.clientX - downAt.x, e.clientY - downAt.y) > 4) {
            downAt = null;
            props.startDrag();
          }
        }}
        onMouseUp={() => {
          if (downAt) props.emit('mascot-open', {});
          downAt = null;
        }}
      >
        <Unicrab state={state()} size={size()} />
        <Show when={count() > 0}>
          <span class="ny-badge mascot-badge">{count() > 9 ? '9+' : count()}</span>
        </Show>
      </button>
    </div>
  );
}

const root = document.getElementById('root');
if (root) {
  document.documentElement.classList.add('mascot-page');
  void (async () => {
    const { emitTo, listen } = await import('@tauri-apps/api/event');
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    const win = getCurrentWindow();
    render(() => {
      onMount(() => {
        const off = listen<MascotFeed>('mascot-feed', (e) => (window as unknown as { __unipiMascotFeed: (f: MascotFeed) => void }).__unipiMascotFeed(e.payload));
        void emitTo('main', 'mascot-ready', {});
        onCleanup(() => void off.then((f) => f()));
      });
      return <MascotWindow emit={(event, payload) => void emitTo('main', event, payload ?? {})} startDrag={() => void win.startDragging()} />;
    }, root);
  })();
}
