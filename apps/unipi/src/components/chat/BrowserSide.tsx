/**
 * The browser beside the chat (UNI-296): the PC's browser (the same live
 * view as Preview → Browser) next to the open chat, for watching — and
 * taking over from — the agent driving it.
 *
 * - Desktop shell: a side pane right of the chat, resizable (drag the edge,
 *   arrow keys, double-click resets; width remembered). The chat layer
 *   shrinks to make room (`html[data-browser-pane]` + `--browser-pane-w`).
 *   When the window can't fit list + chat + pane, the chat list folds away
 *   while the pane is open (`data-browser-pane=fold`): sidebar | chat | browser.
 * - Phone: a full-height sheet over the chat.
 *
 * Opened from the chat header (⋯ → Browser, the globe icon on the desktop)
 * or Ctrl/⌘+Shift+B.
 */
import { createEffect, createSignal, onCleanup, Show } from 'solid-js';
import { BrowserViewer } from '../../screens/BrowserView';
import { CommandBoxInline } from '../CommandBoxInline';
import { browserPane, clampPaneWidth, foldsList, PANE_DEFAULT } from '../../lib/browserPane';
import { SIDEBAR_WIDTH } from '../../lib/layout';

/** The chat area's left edge (the detail pane's left on the desktop shell). */
function areaLeft(): number {
  if (typeof document === 'undefined') return 0;
  const raw = getComputedStyle(document.documentElement).getPropertyValue('--detail-left');
  const n = parseFloat(raw);
  return Number.isFinite(n) ? n : 0;
}

/** Desktop: the pane beside the chat. Renders nothing unless `show`. */
export function BrowserSidePane(props: { show: boolean; hostId: string; hostName: string }) {
  const pane = browserPane();
  const [viewport, setViewport] = createSignal(typeof window !== 'undefined' ? window.innerWidth : 1440);
  const [left, setLeft] = createSignal(areaLeft());
  const onResize = () => {
    setViewport(window.innerWidth);
    setLeft(areaLeft());
  };
  window.addEventListener('resize', onResize);
  onCleanup(() => window.removeEventListener('resize', onResize));
  /** No room for the list beside chat + pane: fold it (the chat starts at the sidebar). */
  const fold = () => foldsList(viewport(), left(), pane.width());
  const areaStart = () => (fold() ? SIDEBAR_WIDTH : left());
  const width = () => clampPaneWidth(pane.width(), viewport() - areaStart());

  createEffect(() => {
    const root = document.documentElement;
    if (!props.show) return;
    // The list splitter moves the chat's left edge: follow it.
    setLeft(areaLeft());
    root.dataset.browserPane = fold() ? 'fold' : '';
    root.style.setProperty('--browser-pane-w', `${width()}px`);
    onCleanup(() => {
      delete root.dataset.browserPane;
      root.style.removeProperty('--browser-pane-w');
    });
  });

  let dragging = false;
  const onDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    dragging = true;
    setLeft(areaLeft());
    (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
    document.documentElement.dataset.resizing = '';
  };
  const onMove = (e: PointerEvent) => {
    if (dragging) pane.setWidth(clampPaneWidth(window.innerWidth - e.clientX, viewport() - areaStart()));
  };
  const onUp = () => {
    dragging = false;
    delete document.documentElement.dataset.resizing;
  };
  const onKey = (e: KeyboardEvent) => {
    const step = e.shiftKey ? 64 : 16;
    if (e.key === 'ArrowLeft') pane.setWidth(clampPaneWidth(width() + step, viewport() - areaStart()));
    else if (e.key === 'ArrowRight') pane.setWidth(clampPaneWidth(width() - step, viewport() - areaStart()));
    else return;
    e.preventDefault();
  };

  return (
    <Show when={props.show}>
      <div
        class="browser-pane-splitter"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the browser"
        aria-valuenow={width()}
        tabindex="0"
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onDblClick={() => pane.setWidth(clampPaneWidth(PANE_DEFAULT, viewport() - areaStart()))}
        onKeyDown={onKey}
        data-testid="browser-pane-splitter"
      />
      <aside class="browser-side-pane" aria-label={`Browser on ${props.hostName}`} data-testid="browser-side-pane">
        <Show when={`${props.hostId}\u0000${pane.profileFor(props.hostId)}`} keyed>
          {(_key: string) => (
            <BrowserViewer
              variant="pane"
              hostId={props.hostId}
              hostName={props.hostName}
              profile={pane.profileFor(props.hostId)}
              onProfile={(name) => pane.setProfileFor(props.hostId, name)}
              CommandBox={CommandBoxInline}
              onClose={() => pane.setOpen(false)}
            />
          )}
        </Show>
      </aside>
    </Show>
  );
}

/** Phone: the browser as a full-screen pane with a ← Back header (never a bottom drawer). */
export function BrowserSheet(props: { open: boolean; hostId: string; hostName: string; onClose: () => void }) {
  const pane = browserPane();
  return (
    <Show when={props.open}>
      <div class="browser-sheet browser-full-pane" role="dialog" aria-modal="true" aria-label={`Browser on ${props.hostName}`} data-testid="browser-sheet">
        <header class="stab-layer-head browser-pane-mobile-head">
          <button type="button" class="ghost-icon-button" aria-label="Back" onClick={() => props.onClose()} data-testid="browser-pane-back">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m12 19-7-7 7-7" /><path d="M19 12H5" /></svg>
          </button>
          <span class="stab-layer-title truncate">Browser · {props.hostName}</span>
        </header>
        <Show when={`${props.hostId}\u0000${pane.profileFor(props.hostId)}`} keyed>
          {(_key: string) => (
            <BrowserViewer
              variant="sheet"
              hostId={props.hostId}
              hostName={props.hostName}
              profile={pane.profileFor(props.hostId)}
              onProfile={(name) => pane.setProfileFor(props.hostId, name)}
              CommandBox={CommandBoxInline}
              onClose={() => props.onClose()}
            />
          )}
        </Show>
      </div>
    </Show>
  );
}
