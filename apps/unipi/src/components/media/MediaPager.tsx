/**
 * Folder media pager (UNI-168): pages through a folder's previewable
 * siblings across media kinds — image → video → pdf → text — loading
 * each on demand through the blob channel. Swipe at the screen edge (or
 * the chevrons) moves to the previous/next file; the edge zone keeps
 * inner gestures (pinch/zoom, code scroll, pdf pan) untouched. Audio and
 * video stop playing when the page turns (their element unmounts).
 */
import { createEffect, createSignal, onCleanup, Show, type JSX } from 'solid-js';
import { ChevronLeft, ChevronRight } from 'lucide-solid';
import { blobTransport } from '../../lib/blob';
import { classifyBytes, type MediaKind } from '../../lib/media/previewer';
import type { PreviewRequest } from '../../lib/media/previewerState';

const EDGE = 34; // px from the left/right edge where a swipe pages
const SWIPE_MIN = 48; // px of horizontal travel before it counts

type LoadedItem =
  | { kind: 'image'; url: string; mime: string; name: string }
  | { kind: Exclude<MediaKind, 'image'>; name: string; mime: string; url?: string; text?: string; reason?: 'too-large' };

/** The core's "too large to preview inline" refusal (blob_get_bytes' 25 MB cap). */
export function isTooLargeToPreview(error: unknown): boolean {
  const e = error as { code?: string; message?: string } | undefined;
  return e?.code === 'E_UNSUPPORTED' && /too large/i.test(e.message ?? '');
}

export function MediaPager(props: {
  hostId: string;
  paths: string[];
  index: number;
  onClose: () => void;
  /** Renders one loaded item (Previewer wires this to its own viewers). */
  render: (request: PreviewRequest, onClose: () => void) => JSX.Element;
}) {
  const [index, setIndex] = createSignal(props.index);
  const [item, setItem] = createSignal<LoadedItem | undefined>(undefined);
  const [error, setError] = createSignal<string | undefined>(undefined);
  let currentUrl: string | undefined;

  onCleanup(() => {
    if (currentUrl) URL.revokeObjectURL(currentUrl);
  });

  createEffect(() => {
    const path = props.paths[index()];
    if (!path) return;
    setError(undefined);
    setItem(undefined);
    void (async () => {
      try {
        const got = await blobTransport().getBytes(props.hostId, { path });
        if (currentUrl) URL.revokeObjectURL(currentUrl);
        currentUrl = undefined;
        const name = path.split('/').pop() ?? path;
        // IPC may hand the bytes over as a number array (see the preview tab's
        // note); a Blob would stringify that instead of reading bytes.
        const raw = got.bytes as unknown;
        const bytes = raw instanceof Uint8Array ? raw : new Uint8Array(raw as ArrayLike<number>);
        const kind = classifyBytes(got.mime, path, bytes);
        const blob = new Blob([bytes as BlobPart], { type: got.mime || 'application/octet-stream' });
        if (kind === 'image' || kind === 'pdf' || kind === 'audio' || kind === 'video' || kind === 'other') {
          currentUrl = URL.createObjectURL(blob);
          setItem(kind === 'image' ? { kind: 'image', url: currentUrl, mime: got.mime, name } : { kind, name, mime: got.mime, url: currentUrl });
        } else {
          setItem({ kind, name, mime: got.mime, text: await blob.text() });
        }
      } catch (e) {
        // UNI-225: a big file isn't an error — show its page with Download / Open with…
        if (isTooLargeToPreview(e)) {
          setItem({ kind: 'other', name: path.split('/').pop() ?? path, mime: 'application/octet-stream', reason: 'too-large' });
          return;
        }
        setError((e as { message?: string })?.message ?? 'Could not load that file');
      }
    })();
  });

  const go = (delta: number) => {
    const next = index() + delta;
    if (next < 0 || next >= props.paths.length) return;
    setIndex(next);
  };

  // Edge-zone swipe: track a touch that STARTS within EDGE px of a
  // screen edge; page when it travels far enough horizontally. Below the
  // header, so close/share buttons keep their taps.
  let swipe: { x0: number; y0: number } | undefined;
  const zoneStart = (e: TouchEvent) => {
    const t = e.touches[0];
    if (!t) return;
    if (t.clientX <= EDGE || t.clientX >= window.innerWidth - EDGE) swipe = { x0: t.clientX, y0: t.clientY };
  };
  const zoneEnd = (e: TouchEvent) => {
    if (!swipe) return;
    const t = e.changedTouches[0];
    const start = swipe;
    swipe = undefined;
    if (!t) return;
    const dx = t.clientX - start.x0;
    const dy = t.clientY - start.y0;
    if (Math.abs(dx) > SWIPE_MIN && Math.abs(dx) > Math.abs(dy) * 1.5) go(dx < 0 ? 1 : -1);
  };

  const request = (): PreviewRequest | undefined => {
    const it = item();
    const path = props.paths[index()];
    if (!it || !path) return undefined;
    if (it.kind === 'image') return { kind: 'image', items: [{ url: it.url, mime: it.mime, name: it.name }], index: 0 };
    return { kind: it.kind, name: it.name, mime: it.mime, url: it.url, text: it.text, reason: it.reason, source: { hostId: props.hostId, path } };
  };

  return (
    <div class="media-pager" data-testid="media-pager">
      <Show when={request()}>{(req) => props.render(req(), props.onClose)}</Show>
      <Show when={!item() && !error()}>
        <div class="media-pager-loading" role="status">Loading…</div>
      </Show>
      <Show when={error()}>
        <div class="media-pager-error" role="alert">{error()}</div>
      </Show>
      <Show when={props.paths.length > 1}>
        <div class="media-pager-pos" data-testid="media-pager-pos">{index() + 1} / {props.paths.length}</div>
        <div class="media-pager-zone media-pager-left" onTouchStart={zoneStart} onTouchEnd={zoneEnd}>
          <button type="button" class="media-pager-nav" aria-label="Previous file" disabled={index() === 0} onClick={() => go(-1)}>
            <ChevronLeft size={26} />
          </button>
        </div>
        <div class="media-pager-zone media-pager-right" onTouchStart={zoneStart} onTouchEnd={zoneEnd}>
          <button type="button" class="media-pager-nav" aria-label="Next file" disabled={index() === props.paths.length - 1} onClick={() => go(1)}>
            <ChevronRight size={26} />
          </button>
        </div>
      </Show>
    </div>
  );
}
