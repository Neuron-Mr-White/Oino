/**
 * `Image` (ui-protocol §4.5.2): `<img loading="lazy">`. Inline `data` renders
 * as a data URL; `{blob}` / `{path}` are fetched through the host blob channel
 * (`blob_get`) only once the image scrolls into view (§13.5 rule), then shown
 * from an object URL. Click opens the media previewer. `maxWidth`/`maxHeight`
 * in cells (≤ 20 rows by default); max width 100 % on the phone.
 */
import { Show, createSignal, onCleanup, onMount, type JSX } from 'solid-js';
import { ImageOff } from 'lucide-solid';
import { blobTransport } from '../../lib/blob';
import { openPreviewer } from '../../lib/media/previewerState';
import { useEntry } from '../actions';
import type { ImageNode, ImageSource } from '../types';
import { baseStyle, cx, type NodeProps } from './props';

const SAFE_MIME = /^image\/(png|jpe?g|gif|webp|avif|bmp|svg\+xml)$/i;

function dataUrl(src: ImageSource): string | undefined {
  if ('data' in src && typeof src.data === 'string' && SAFE_MIME.test(src.mimeType ?? '')) return `data:${src.mimeType};base64,${src.data}`;
  return undefined;
}

export function Image(props: NodeProps<ImageNode>) {
  const entry = useEntry();
  let el!: HTMLDivElement;
  const [url, setUrl] = createSignal<string | undefined>(dataUrl(props.node.src ?? ({} as ImageSource)));
  const [failed, setFailed] = createSignal(false);
  let objectUrl: string | undefined;
  onCleanup(() => objectUrl && URL.revokeObjectURL(objectUrl));

  const fetchRemote = async () => {
    const src = props.node.src;
    const hostId = entry.target.hostId;
    if (!src || !hostId) return setFailed(true);
    const ref = 'blob' in src ? { sha256: src.blob } : 'path' in src ? { path: src.path } : undefined;
    if (!ref) return setFailed(true);
    try {
      const got = await blobTransport().getBytes(hostId, ref);
      objectUrl = URL.createObjectURL(new Blob([got.bytes as BlobPart], { type: got.mime || 'image/png' }));
      setUrl(objectUrl);
    } catch {
      setFailed(true);
    }
  };

  onMount(() => {
    if (url() || !props.node.src) return;
    if (typeof IntersectionObserver === 'undefined') return void fetchRemote();
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        io.disconnect();
        void fetchRemote();
      }
    });
    io.observe(el);
    onCleanup(() => io.disconnect());
  });

  const box = (): JSX.CSSProperties => ({
    ...baseStyle(props.node).style,
    'max-width': props.node.maxWidth ? `min(100%, calc(${props.node.maxWidth} * var(--ui-col)))` : '100%',
    'max-height': `calc(${props.node.maxHeight ?? 20} * var(--ui-row))`,
  });
  const open = () => {
    const u = url();
    if (!u) return;
    openPreviewer({ kind: 'image', items: [{ url: u, mime: 'image/*', name: props.node.alt || 'image' }], index: 0 });
  };

  return (
    <div ref={el} class={cx('ui-image', baseStyle(props.node).class)} data-fit={(props.node.fit ?? 'Contain').toLowerCase()} style={box()}>
      <Show
        when={url() && !failed()}
        fallback={
          <div class="ui-image-ph" role="img" aria-label={props.node.alt}>
            <Show when={failed()}>
              <ImageOff size={16} aria-hidden="true" />
            </Show>
            <span>{props.node.alt || 'Image'}</span>
          </div>
        }
      >
        <button type="button" class="ui-image-btn" onClick={open} aria-label={`Open image: ${props.node.alt}`}>
          <img src={url()} alt={props.node.alt} loading="lazy" decoding="async" on:error={() => setFailed(true)} />
        </button>
      </Show>
    </div>
  );
}
