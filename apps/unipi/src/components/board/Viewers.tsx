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
import { createEffect, onCleanup, onMount, Show } from 'solid-js';
import { Portal } from 'solid-js/web';
import { ArrowLeft, Copy } from 'lucide-solid';
import { api } from '../../lib/board/api';
import { relativeTime } from '../../lib/board/markdown';
import { boardStore } from '../../lib/board/store';
import { boardUi, type ReaderEntry } from '../../lib/board/ui';
import { pushBack } from '../../lib/back';
import { copyToClipboard } from '../../lib/board/taskActions';
import { openPreviewer } from '../../lib/media/previewerState';
import { Markdown } from './Markdown';

export function ReportReader() {
  const ui = boardUi();
  return (
    <Show when={ui.reader()}>
      {(entry) => <ReaderPage entry={entry()} onClose={() => ui.setReader(null)} />}
    </Show>
  );
}

function ReaderPage(props: { entry: ReaderEntry; onClose: () => void }) {
  onMount(() => onCleanup(pushBack(props.onClose)));
  return (
    <Portal>
      <div class="board-page reader-page" role="dialog" aria-label={props.entry.title} data-testid="report-reader">
        <header class="task-head">
          <button type="button" class="ghost-icon-button" aria-label="Close reader" onClick={props.onClose}>
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
export function ImageViewer() {
  const ui = boardUi();
  const store = boardStore();
  createEffect(() => {
    const request = ui.viewer();
    if (!request) return;
    const hostId = store.target()?.hostId;
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
      openPreviewer({ kind: 'image', items, index });
    });
  });
  return null;
}
