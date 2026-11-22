/**
 * Full-screen readers (UNI-131):
 *  - ReportReader — the web's ReportReader: one report / note / text
 *    file in full, as its own page over the detail (Back closes), with
 *    Copy;
 *  - ImageViewer — a bridge to the shared previewer (UNI-157,
 *    components/media/Previewer.tsx): resolves every item's path to an
 *    object URL (board transport) and opens it there, so board images get
 *    the same pinch/pan/swipe-between-images/share viewer as chat ones,
 *    with no separate gesture code to keep in sync.
 */
import { createEffect, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { ArrowLeft, ChevronLeft, ChevronRight, Copy, X } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { api } from '../../lib/board/api';
import { relativeTime } from '../../lib/board/markdown';
import { boardStore } from '../../lib/board/store';
import { boardUi, type ReaderEntry } from '../../lib/board/ui';
import { pushBack } from '../../lib/back';
import { copyToClipboard } from '../../lib/board/taskActions';
import { openPreviewer } from '../../lib/media/previewerState';
import { Markdown } from './Markdown';
import { ReportMarkdown } from './ReportMarkdown';
import { wrapTab } from '../../lib/focusTrap';

/**
 * `pane`: the desktop Workbench mounts the reader as a column beside the
 * ticket (it never covers the reply dock); otherwise it is a page over the
 * detail, as on the Board and the phone.
 */
export function ReportReader(props: { mount?: HTMLElement; pane?: boolean } = {}) {
  const ui = boardUi();
  return (
    <Show when={ui.reader()}>
      {(entry) => (props.pane && props.mount
        ? <PaneReader entry={entry()} mount={props.mount} onClose={() => ui.setReader(null)} />
        : <ReaderPage entry={entry()} mount={props.mount} onClose={() => ui.setReader(null)} />)}
    </Show>
  );
}

/** The side pane's header: title · Copy · Close (Esc). */
function PaneHead(props: { title: string; meta?: string; onClose: () => void; onCopy?: () => void; children?: any }) {
  return (
    <header class="sa-pane-head">
      <div class="min-w-0 flex-1">
        <p class="reader-title truncate">{props.title}</p>
        <Show when={props.meta}><p class="reader-meta truncate">{props.meta}</p></Show>
      </div>
      {props.children}
      <Show when={props.onCopy}>
        <button type="button" class="sa-pane-btn" aria-label="Copy text" onClick={props.onCopy}><Copy size={15} /> Copy</button>
      </Show>
      <button type="button" class="sa-pane-btn sa-pane-close" aria-label="Close reader" title="Close (Esc)" onClick={props.onClose}><X size={16} /></button>
    </header>
  );
}

function PaneReader(props: { entry: ReaderEntry; mount: HTMLElement; onClose: () => void }) {
  onMount(() => onCleanup(pushBack(props.onClose)));
  return (
    <Portal mount={props.mount}>
      <section class="sa-pane-view reader-pane" aria-label={props.entry.title} data-testid="report-reader" onKeyDown={(e) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); props.onClose(); } }}>
        <PaneHead title={props.entry.title} meta={[props.entry.actor, relativeTime(props.entry.at)].filter(Boolean).join(' · ')} onClose={props.onClose} onCopy={() => void copyToClipboard(props.entry.text, 'the text')} />
        <div class="board-page-scroll reader-body">
          <Show when={props.entry.mono} fallback={<ReportMarkdown text={props.entry.text} class="md-reader" />}>
            <pre class="reader-pre">{props.entry.text}</pre>
          </Show>
        </div>
      </section>
    </Portal>
  );
}

function ReaderPage(props: { entry: ReaderEntry; mount?: HTMLElement; onClose: () => void }) {
  let closeButton: HTMLButtonElement | undefined;
  onMount(() => {
    const previous = document.activeElement as HTMLElement | null;
    if (props.mount) closeButton?.focus({ preventScroll: true });
    onCleanup(pushBack(props.onClose));
    onCleanup(() => { if (props.mount && previous?.isConnected) previous.focus({ preventScroll: true }); });
  });
  return (
    <Portal mount={props.mount}>
      <div class="board-page reader-page" role="dialog" aria-label={props.entry.title} data-testid="report-reader" onKeyDown={props.mount ? (e) => {
        if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); props.onClose(); }
        else if (e.key === 'Tab') { wrapTab(e, e.currentTarget); e.stopPropagation(); }
        else e.stopPropagation();
      } : undefined}>
        <header class="task-head">
          <button ref={closeButton} type="button" class="ghost-icon-button" aria-label="Close reader" onClick={props.onClose}>
            <ArrowLeft size={22} />
          </button>
          <div class="min-w-0 flex-1">
            <p class="reader-title truncate">{props.entry.title}</p>
            <Show when={props.entry.actor || props.entry.at}>
              <p class="reader-meta truncate">
                {[props.entry.actor, relativeTime(props.entry.at)].filter(Boolean).join(' · ')}
              </p>
            </Show>
          </div>
          <button type="button" class="ghost-icon-button" aria-label="Copy text" onClick={() => void copyToClipboard(props.entry.text, 'the text')}>
            <Copy size={19} />
          </button>
        </header>
        <div class="board-page-scroll reader-body">
          <Show when={props.entry.mono} fallback={<Markdown text={props.entry.text} class="md-reader" />}>
            <pre class="reader-pre">{props.entry.text}</pre>
          </Show>
        </div>
      </div>
    </Portal>
  );
}

/** `ui.viewer()` → the shared previewer, resolving every item's board path
 * to an object URL first. Renders nothing itself (no `<Show>` markup): the
 * previewer is already mounted once at the app root (main.tsx). */
export function ImageViewer(props: { mount?: HTMLElement; pane?: boolean } = {}) {
  const ui = boardUi();
  const store = boardStore();
  const inPane = () => !!props.pane && !!props.mount && layout().desktop();
  // Something else took the pane (a report, a text file): the image gives way.
  createEffect(() => { if (ui.reader()) ui.setPaneImage(null); });
  createEffect(() => {
    const request = ui.viewer();
    if (!request) return;
    const hostId = store.target()?.hostId;
    const pane = inPane();
    ui.setViewer(null);
    if (!hostId) return;
    void Promise.all(
      request.items.map(async (item) => {
        try {
          const url = await api.fileUrl(hostId, item.path);
          return { url, mime: 'image/*', name: item.name };
        } catch {
          return undefined;
        }
      }),
    ).then((resolved) => {
      const items = resolved.filter((item): item is { url: string; mime: string; name: string } => !!item);
      if (!items.length) return;
      const index = Math.min(request.index, items.length - 1);
      if (pane) {
        ui.setReader(null);
        ui.setPaneImage({ items: items.map(({ url, name }) => ({ url, name })), index });
      } else openPreviewer({ kind: 'image', items, index });
    });
  });
  return (
    <Show when={inPane() && ui.paneImage()}>
      {(image) => <PaneImagePage image={image()} mount={props.mount!} onClose={() => ui.setPaneImage(null)} onIndex={(index) => ui.setPaneImage({ ...image(), index })} />}
    </Show>
  );
}

/** One image (paging between the ticket's images) in the side pane. */
function PaneImagePage(props: { image: { items: { url: string; name: string }[]; index: number }; mount: HTMLElement; onClose: () => void; onIndex: (index: number) => void }) {
  const [failed, setFailed] = createSignal(false);
  const at = () => Math.min(props.image.index, props.image.items.length - 1);
  const shown = () => props.image.items[at()]!;
  const step = (delta: number) => { setFailed(false); props.onIndex(Math.max(0, Math.min(props.image.items.length - 1, at() + delta))); };
  onMount(() => onCleanup(pushBack(props.onClose)));
  return (
    <Portal mount={props.mount}>
      <section class="sa-pane-view image-pane" aria-label={shown().name} data-testid="pane-image" tabindex={-1} onKeyDown={(e) => {
        if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); props.onClose(); }
        else if (e.key === 'ArrowLeft') { e.preventDefault(); step(-1); }
        else if (e.key === 'ArrowRight') { e.preventDefault(); step(1); }
      }}>
        <PaneHead title={shown().name} meta={props.image.items.length > 1 ? `${at() + 1} of ${props.image.items.length}` : undefined} onClose={props.onClose}>
          <Show when={props.image.items.length > 1}>
            <button type="button" class="sa-pane-btn" aria-label="Previous image" disabled={at() === 0} onClick={() => step(-1)}><ChevronLeft size={16} /></button>
            <button type="button" class="sa-pane-btn" aria-label="Next image" disabled={at() >= props.image.items.length - 1} onClick={() => step(1)}><ChevronRight size={16} /></button>
          </Show>
        </PaneHead>
        <div class="image-pane-stage">
          <Show when={!failed()} fallback={<p class="sa-empty">This image could not be shown.</p>}>
            <img src={shown().url} alt={shown().name} onError={() => setFailed(true)} />
          </Show>
        </div>
      </section>
    </Portal>
  );
}
