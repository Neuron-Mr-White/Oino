/**
 * Full-screen previewer (UNI-157): images (pinch/pan/swipe, lifted from
 * components/board/Viewers.tsx's ImageViewer), text/markdown/code (shared
 * highlighter), logs (monospace tail), audio/video, and PDF ("Open with…"
 * share — no bundled PDF renderer is light enough for the WebView yet).
 * Opened via `openPreviewer()` (lib/media/previewerState.ts).
 */
import { PdfView } from './PdfView';
import { createEffect, createResource, createSignal, For, Match, on, onCleanup, onMount, Show, Switch } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { Portal } from 'solid-js/web';
import { ArrowLeft, ChevronLeft, ChevronRight, Download, Eye, FileText, Share2, X } from 'lucide-solid';
import { pushBack } from '../../lib/back';
import { saveFile } from '../../lib/board/share';
import { downloadWithNotify, openWithExternal, progressLabel, transferMessage, type TransferProgress } from '../../lib/filesTransfer';
import { setEditorTarget } from '../../lib/editorTarget';
import { escapeHtml } from '../../lib/markdown/escape';
import { highlightHtml } from '../../lib/markdown/highlight';
import { renderBlocks } from '../../lib/chat/markdown';
import { MdBlock } from '../chat/MdBlock';
import { closePreviewer, previewerRequest, type PreviewRequest } from '../../lib/media/previewerState';
import { classifyMedia, isKnownBinaryExt, languageOf } from '../../lib/media/previewer';
import { OpenMenu } from '../files/OpenMenu';
import { EmptyState } from '../EmptyState';
import { isOwnPcFile } from '../../lib/openLocal';
import { openSideTab } from '../panes/SidePane';
import { currentWorkspace } from '../shell/Workspace';
import { switchEditorTarget } from '../../lib/editorTarget';
import { blobTransport } from '../../lib/blob';
import { fsRead } from '../../lib/fs';
import { setPaneComponent, type PaneTabProps } from '../../lib/panes/kinds';
import { MediaPager } from './MediaPager';
import { desktopShell } from '../../lib/layout';

export function Previewer() {
  const req = previewerRequest();
  return (
    <Show when={req()}>
      {(r) => <PreviewerLayer request={r()} onClose={closePreviewer} />}
    </Show>
  );
}

type Xform = { scale: number; x: number; y: number };
const MAX_SCALE = 5;
const clampN = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

function PreviewerLayer(props: { request: PreviewRequest; onClose: () => void }) {
  onMount(() => onCleanup(pushBack(props.onClose)));
  return (
    <Portal>
      <Show
        when={props.request.kind === 'list' ? (props.request as Extract<PreviewRequest, { kind: 'list' }>) : undefined}
        fallback={
          <Show
            when={props.request.kind === 'image' ? (props.request as Extract<PreviewRequest, { kind: 'image' }>) : undefined}
            fallback={<GenericPreview request={props.request as Exclude<PreviewRequest, { kind: 'image' | 'list' }>} onClose={props.onClose} />}
          >
            {(imageRequest) => <ImagePreview request={imageRequest()} onClose={props.onClose} />}
          </Show>
        }
      >
        {(listRequest) => (
          <MediaPager
            hostId={listRequest().hostId}
            paths={listRequest().paths}
            index={listRequest().index}
            onClose={props.onClose}
            render={(request, onClose) => (
              <Show
                when={request.kind === 'image' ? (request as Extract<PreviewRequest, { kind: 'image' }>) : undefined}
                fallback={<GenericPreview request={request as Exclude<PreviewRequest, { kind: 'image' | 'list' }>} onClose={onClose} />}
              >
                {(imageRequest) => <ImagePreview request={imageRequest()} onClose={onClose} />}
              </Show>
            )}
          />
        )}
      </Show>
    </Portal>
  );
}

export function ImagePreview(props: { request: Extract<PreviewRequest, { kind: 'image' }>; onClose: () => void }) {
  const [index, setIndex] = createSignal(props.request.index);
  const [xf, setXf] = createSignal<Xform>({ scale: 1, x: 0, y: 0 });
  const [drag, setDrag] = createSignal({ x: 0, y: 0 });
  const [animate, setAnimate] = createSignal(true);
  let stage: HTMLDivElement | undefined;
  const item = () => props.request.items[index()];
  const many = () => props.request.items.length > 1;

  createEffect(on(index, () => setXf({ scale: 1, x: 0, y: 0 })));

  const go = (delta: number) => {
    const next = index() + delta;
    if (next < 0 || next >= props.request.items.length) return;
    setIndex(next);
  };

  const bounds = (scale: number) => {
    const rect = stage?.getBoundingClientRect();
    const w = rect?.width ?? window.innerWidth;
    const h = rect?.height ?? window.innerHeight;
    return { x: ((scale - 1) * w) / 2, y: ((scale - 1) * h) / 2, w, h, left: rect?.left ?? 0, top: rect?.top ?? 0 };
  };
  const clamp = (next: Xform): Xform => {
    const b = bounds(next.scale);
    return { scale: next.scale, x: clampN(next.x, -b.x, b.x), y: clampN(next.y, -b.y, b.y) };
  };
  const zoomAt = (scale: number, cx: number, cy: number, from = xf()) => {
    const b = bounds(scale);
    const mx = cx - b.left - b.w / 2;
    const my = cy - b.top - b.h / 2;
    const px = (mx - from.x) / from.scale;
    const py = (my - from.y) / from.scale;
    return clamp({ scale, x: mx - px * scale, y: my - py * scale });
  };

  type Gesture =
    | { kind: 'pinch'; dist: number; start: Xform; cx: number; cy: number }
    | { kind: 'pan'; x0: number; y0: number; start: Xform; t0: number }
    | { kind: 'none' };
  let gesture: Gesture = { kind: 'none' };
  let lastTap = { at: 0, x: 0, y: 0 };
  const dist = (touches: TouchList) => Math.hypot(touches[0]!.clientX - touches[1]!.clientX, touches[0]!.clientY - touches[1]!.clientY);
  const onTouchStart = (event: TouchEvent) => {
    setAnimate(false);
    if (event.touches.length === 2) {
      gesture = { kind: 'pinch', dist: dist(event.touches), start: xf(), cx: (event.touches[0]!.clientX + event.touches[1]!.clientX) / 2, cy: (event.touches[0]!.clientY + event.touches[1]!.clientY) / 2 };
      setDrag({ x: 0, y: 0 });
    } else if (event.touches.length === 1) {
      gesture = { kind: 'pan', x0: event.touches[0]!.clientX, y0: event.touches[0]!.clientY, start: xf(), t0: Date.now() };
    }
  };
  const onTouchMove = (event: TouchEvent) => {
    if (gesture.kind === 'pinch' && event.touches.length === 2) {
      event.preventDefault();
      const scale = clampN((gesture.start.scale * dist(event.touches)) / gesture.dist, 1, MAX_SCALE);
      const cx = (event.touches[0]!.clientX + event.touches[1]!.clientX) / 2;
      const cy = (event.touches[0]!.clientY + event.touches[1]!.clientY) / 2;
      const zoomed = zoomAt(scale, gesture.cx, gesture.cy, gesture.start);
      setXf(clamp({ scale, x: zoomed.x + (cx - gesture.cx), y: zoomed.y + (cy - gesture.cy) }));
    } else if (gesture.kind === 'pan' && event.touches.length === 1) {
      event.preventDefault();
      const dx = event.touches[0]!.clientX - gesture.x0;
      const dy = event.touches[0]!.clientY - gesture.y0;
      if (gesture.start.scale > 1.01) setXf(clamp({ ...gesture.start, x: gesture.start.x + dx, y: gesture.start.y + dy }));
      else setDrag(Math.abs(dx) > Math.abs(dy) ? { x: dx, y: 0 } : { x: 0, y: Math.max(0, dy) });
    }
  };
  const onTouchEnd = (event: TouchEvent) => {
    setAnimate(true);
    if (gesture.kind === 'pan' && event.touches.length === 0) {
      const touch = event.changedTouches[0];
      const dx = (touch?.clientX ?? gesture.x0) - gesture.x0;
      const dy = (touch?.clientY ?? gesture.y0) - gesture.y0;
      const zoomed = gesture.start.scale > 1.01;
      const tap = Math.abs(dx) < 10 && Math.abs(dy) < 10 && Date.now() - gesture.t0 < 300;
      if (tap) {
        const now = Date.now();
        const x = touch?.clientX ?? 0;
        const y = touch?.clientY ?? 0;
        if (now - lastTap.at < 300 && Math.hypot(x - lastTap.x, y - lastTap.y) < 30) {
          setXf(xf().scale > 1.01 ? { scale: 1, x: 0, y: 0 } : zoomAt(2.5, x, y));
          lastTap = { at: 0, x: 0, y: 0 };
        } else lastTap = { at: now, x, y };
      } else if (!zoomed) {
        if (Math.abs(dx) > 60 && Math.abs(dx) > Math.abs(dy)) go(dx < 0 ? 1 : -1);
        else if (dy > 110 && dy > Math.abs(dx)) props.onClose();
      }
      setDrag({ x: 0, y: 0 });
    }
    if (event.touches.length === 0) {
      gesture = { kind: 'none' };
      if (xf().scale < 1.02) setXf({ scale: 1, x: 0, y: 0 });
    } else if (event.touches.length === 1) {
      gesture = { kind: 'pan', x0: event.touches[0]!.clientX, y0: event.touches[0]!.clientY, start: xf(), t0: 0 };
    }
  };

  async function share() {
    const current = item();
    if (!current) return;
    try {
      const response = await fetch(current.url);
      const blob = await response.blob();
      const name = current.name ?? `image.${current.mime.split('/')[1] ?? 'jpg'}`;
      await saveFile(blob, name);
    } catch {
      // best effort
    }
  }

  const backdrop = () => Math.max(0.35, 1 - drag().y / 400);
  return (
    <div class="image-viewer" role="dialog" aria-label="Image" data-testid="media-previewer" style={{ '--viewer-bg': backdrop() }}>
      <header class="image-viewer-head detail-head">
        <button type="button" class="viewer-icon" aria-label="Close image" onClick={props.onClose}>
          <X size={22} />
        </button>
        <div class="min-w-0 flex-1">
          <Show when={many()}>
            <p class="text-[12px] text-text-muted tabular-nums">{index() + 1} / {props.request.items.length}</p>
          </Show>
        </div>
        <button type="button" class="viewer-icon" aria-label="Share or save" onClick={() => void share()}>
          <Share2 size={20} />
        </button>
      </header>
      <div
        ref={stage}
        class="image-viewer-stage"
        onTouchStart={onTouchStart}
        onTouchMove={onTouchMove}
        onTouchEnd={onTouchEnd}
        onTouchCancel={onTouchEnd}
        onDblClick={(event) => setXf(xf().scale > 1.01 ? { scale: 1, x: 0, y: 0 } : zoomAt(2.5, event.clientX, event.clientY))}
      >
        <Show when={item()}>
          {(current) => (
            <img
              src={current().url}
              alt=""
              draggable={false}
              class="image-viewer-img"
              classList={{ animate: animate() }}
              style={{ transform: `translate(${xf().x + drag().x}px, ${xf().y + drag().y}px) scale(${xf().scale})` }}
            />
          )}
        </Show>
      </div>
      <Show when={many()}>
        <footer class="image-viewer-foot">
          <button type="button" class="viewer-icon" aria-label="Previous image" disabled={index() === 0} onClick={() => go(-1)}>
            <ChevronLeft size={22} />
          </button>
          <div class="viewer-dots">
            <For each={props.request.items}>{(_, at) => <span class="viewer-dot" data-on={at() === index() ? '' : undefined} />}</For>
          </div>
          <button type="button" class="viewer-icon" aria-label="Next image" disabled={index() === props.request.items.length - 1} onClick={() => go(1)}>
            <ChevronRight size={22} />
          </button>
        </footer>
      </Show>
    </div>
  );
}

export function GenericPreview(props: { request: Exclude<PreviewRequest, { kind: 'image' | 'list' }>; onClose: () => void }) {
  const req = props.request;
  const navigate = useNavigate();
  const [toast, setToast] = createSignal<string | null>(null);
  // UNI-191: "Edit as text" for any file type — opens the editor even
  // for a binary/no-preview file, lossily decoded, read-only until the
  // user taps "Edit anyway".
  function editAsText() {
    if (!req.source) return;
    setEditorTarget({ hostId: req.source.hostId, path: req.source.path, asText: true });
    props.onClose();
    navigate('/files/edit');
  }
  const flash = (text: string) => {
    setToast(text);
    setTimeout(() => setToast((t) => (t === text ? null : t)), 2500);
  };
  const run = async (op: 'download' | 'openWith') => {
    if (!req.source) return;
    // UNI-225: streamed (no preview size cap), progress in the toast.
    const verb = op === 'download' ? 'Downloading' : 'Opening';
    setToast(progressLabel(verb, undefined));
    const onProgress = (p: TransferProgress) => setToast(progressLabel(verb, p));
    const result = op === 'download'
      ? await downloadWithNotify(req.source.hostId, req.source.path, onProgress)
      : await openWithExternal(req.source.hostId, req.source.path, onProgress);
    flash(transferMessage(result, req.name, desktopShell()));
  };
  return (
    <div class="media-page" role="dialog" aria-label={req.name} data-testid="media-previewer">
      <header class="media-page-head detail-head">
        <button type="button" class="ghost-icon-button" aria-label="Close" onClick={props.onClose}>
          <ArrowLeft size={22} />
        </button>
        <div class="min-w-0 flex-1">
          <p class="media-page-title truncate">{req.name}</p>
        </div>
        <Show when={req.source}>
          <button type="button" class="ghost-icon-button" aria-label="Edit as text" onClick={editAsText}>
            <FileText size={19} />
          </button>
          <Show
            when={isOwnPcFile(req.source!.hostId)}
            fallback={
              <>
                <button type="button" class="ghost-icon-button" aria-label={desktopShell() ? 'Download' : 'Download to phone'} onClick={() => void run('download')}>
                  <Download size={19} />
                </button>
                <button type="button" class="ghost-icon-button" aria-label="Open with another app" onClick={() => void run('openWith')}>
                  <Share2 size={19} />
                </button>
              </>
            }
          >
            <OpenMenu path={req.source!.path} variant="icon" onResult={flash} />
          </Show>
        </Show>
        <Show when={!req.source && req.url}>
          <a class="ghost-icon-button" aria-label="Share or open with…" href={req.url} target="_blank" rel="noreferrer">
            <Share2 size={19} />
          </a>
        </Show>
      </header>
      <div class="media-page-scroll">
        <Show when={req.kind === 'other'}>
          <EmptyState
            icon={<FileText size={24} />}
            title={req.reason === 'too-large' ? 'Too large to preview here' : 'No preview for this file type'}
            body={req.reason === 'too-large' ? req.name : `${req.name}${desktopShell() ? '' : ' — open it in another app, or download it to your phone.'}`}
            testId="media-nopreview"
            action={
              <Show when={req.source} fallback={<p class="text-text-dim text-sm">Available from the Files tab.</p>}>
                {(source) => (
                  <>
                    <button type="button" class="btn btn-secondary" onClick={editAsText}>Edit as text</button>
                    <Show
                      when={isOwnPcFile(source().hostId)}
                      fallback={
                        <>
                          <button type="button" class="btn btn-secondary" onClick={() => void run('openWith')}>Open with…</button>
                          <button type="button" class="btn btn-secondary" onClick={() => void run('download')}>Download</button>
                        </>
                      }
                    >
                      <OpenMenu path={source().path} variant="button" onResult={flash} />
                    </Show>
                  </>
                )}
              </Show>
            }
          />
        </Show>
        <Show when={req.kind === 'markdown'}>
          <div class="chat-prose md" data-testid="media-md">
            <For each={renderBlocks(req.text ?? '', false)}>{(html) => <MdBlock html={html} class="chat-block" />}</For>
          </div>
        </Show>
        <Show when={req.kind === 'code'}>
          <CodeBlock text={req.text ?? ''} lang={req.mime} />
        </Show>
        <Show when={req.kind === 'log' || req.kind === 'text'}>
          <pre class="reader-pre">{req.text ?? ''}</pre>
        </Show>
        <Show when={req.kind === 'pdf'}>
          <Show when={req.url} fallback={<p class="text-text-muted text-sm p-4">No preview available. Share to open with another app.</p>}>
            {(url) => <PdfView url={url()} name={req.name} />}
          </Show>
        </Show>
        <Show when={req.kind === 'audio' && req.url}>{(url) => <audio controls src={url()} class="media-audio" />}</Show>
        <Show when={req.kind === 'video' && req.url}>{(url) => <video controls src={url()} class="media-video" />}</Show>
      </div>
      <Show when={toast()}>
        <div class="media-toast" role="status">{toast()}</div>
      </Show>
    </div>
  );
}

function CodeBlock(props: { text: string; lang: string }) {
  const [html] = createResource(
    () => [props.text, props.lang] as const,
    async ([text, lang]) => (await highlightHtml(text, lang)) ?? escapeHtml(text),
  );
  return <pre class="reader-pre reader-code" innerHTML={html() ?? escapeHtml(props.text)} />;
}

// ── Preview tab (WP-A20, Z-103) ─────────────────────────────────────────────

/**
 * The `preview` pane tab: the same Previewer content hosted inside the
 * side pane — images on a checkerboard, PDF, audio/video, markdown and
 * SVG with a **preview ↔ source** toggle; no Office (the no-preview card
 * with Edit-as-text / Download instead). The phone keeps the full-screen
 * layer (`openPreviewer`); this is the desktop tab (§3.9). On the phone
 * shell the tab body is never opened, but renders fine if it ever is.
 */
export function PreviewTab(props: PaneTabProps) {
  const hostId = () => String(props.tab.params?.hostId ?? props.hostId);
  const path = () => String(props.tab.params?.path ?? '');
  const name = () => path().slice(path().lastIndexOf('/') + 1) || 'Preview';
  // The extension alone decides text-vs-bytes (`readText`), so the load
  // effect depends on the path only. The rendered kind prefers the blob's
  // real mime once it lands; `.svg` stays `svg` so the source toggle works.
  const TEXT_KINDS = new Set(['markdown', 'code', 'log', 'text']);
  // Unknown extensions (.lock, dotfiles…) are sniffed by content: try text first.
  const readText = () => {
    const k = classifyMedia(undefined, path());
    return TEXT_KINDS.has(k) || (k === 'other' && !isKnownBinaryExt(path()));
  };
  const kind = () => {
    if (path().toLowerCase().endsWith('.svg')) return 'svg';
    const mime = bytes()?.mime;
    return mime ? classifyMedia(mime, path()) : classifyMedia(undefined, path());
  };
  const [source, setSource] = createSignal(false);
  const [sniffedBinary, setSniffedBinary] = createSignal(false);
  const [bytes, setBytes] = createSignal<{ url: string; mime: string } | undefined>();
  const [text, setText] = createSignal<string>();
  const [state, setState] = createSignal<'loading' | 'ready' | 'error'>();
  const [error, setError] = createSignal<string>();
  const [toast, setToast] = createSignal<string | null>(null);
  const flash = (t: string) => {
    setToast(t);
    setTimeout(() => setToast((cur) => (cur === t ? null : t)), 2500);
  };

  createEffect(() => {
    const p = path();
    if (!p) return;
    setState('loading');
    setBytes((b) => (b ? (URL.revokeObjectURL(b.url), undefined) : undefined));
    setText(undefined);
    setSniffedBinary(false);
    setSource(false);
    const token = {};
    current = token;
    void (async () => {
      try {
        if (!readText()) {
          const got = await blobTransport().getBytes(hostId(), { path: p });
          if (token !== current) return;
          const mime = got.mime || 'application/octet-stream';
          // IPC may hand the bytes over as a number array (the mock host and
          // a plain `Vec<u8>` both do); a Blob would stringify that.
          const raw = got.bytes as unknown;
          const bytes = raw instanceof Uint8Array ? raw : new Uint8Array(raw as ArrayLike<number>);
          const blob = new Blob([bytes as BlobPart], { type: mime });
          setBytes({ url: URL.createObjectURL(blob), mime });
        } else {
          const got = await fsRead(hostId(), p);
          if (token !== current) return;
          if (!got.ok && got.error.code === 'E_BINARY') {
            // Sniffed as a true binary: show the no-preview page (bytes not needed).
            setText(undefined);
            setSniffedBinary(true);
          } else if (!got.ok) throw new Error(got.error.message);
          else setText(got.value.text);
        }
        setState('ready');
      } catch (e) {
        if (token !== current) return;
        setError((e as Error).message || 'Could not load this file');
        setState('error');
      }
    })();
  });
  let current: object | undefined;

  async function download() {
    setToast('Downloading…');
    const result = await downloadWithNotify(hostId(), path());
    flash(transferMessage(result, name(), desktopShell()));
  }
  // "Edit as text": the editor lives in the Files tab, so open (or focus) it
  // there with the target set, then drop this preview tab.
  function editAsText() {
    switchEditorTarget({ hostId: hostId(), path: path(), asText: true });
    const ws = currentWorkspace();
    const scope = ws?.scope ?? { hostId: hostId(), projectKey: props.projectKey };
    openSideTab(scope, ws?.view ?? { sessionId: props.sessionId }, { kind: 'files', params: { hostId: hostId(), root: props.projectKey || '~', path: path() } });
    props.close();
  }
  const ownPc = () => isOwnPcFile(hostId());

  const toggleSource = () => {
    if (kind() === 'svg' && !source() && text() === undefined) {
      void fsRead(hostId(), path()).then((got) => got.ok && setText(got.value.text));
    }
    setSource(!source());
  };

  return (
    <div class="ptab" data-testid="preview-tab" data-kind={kind()}>
      <div class="ptab-bar" role="toolbar" aria-label="Preview tools">
        <span class="ptab-name" title={path()}>{name()}</span>
        <span class="ftree-bar-fill" />
        <Show when={kind() === 'markdown' || kind() === 'svg'}>
          <button type="button" class={`ftree-tool${source() ? ' on' : ''}`} aria-pressed={source()} aria-label={source() ? 'Preview' : 'View source'} data-testid="preview-tab-source" onClick={toggleSource}>
            <Show when={source()} fallback={<FileText size={15} />}>
              <Eye size={15} />
            </Show>
          </button>
        </Show>
        <Show when={state() === 'ready'}>
          <Show
            when={ownPc()}
            fallback={
              <button type="button" class="ftree-tool" aria-label="Download" onClick={() => void download()}>
                <Download size={15} />
              </button>
            }
          >
            <OpenMenu path={path()} variant="icon" onResult={flash} />
          </Show>
        </Show>
      </div>
      <div class="ptab-body">
        <Show when={state() === 'loading'}>
          <p class="ptab-note">Loading…</p>
        </Show>
        <Show when={state() === 'error'}>
          <p class="ptab-note" role="alert" data-testid="preview-tab-error">{error()}</p>
        </Show>
        <Show when={state() === 'ready'}>
          <Switch>
            <Match when={(kind() === 'image' || (kind() === 'svg' && !source())) && bytes()}>
              <div class="ptab-img">
                <img src={bytes()!.url} alt={name()} data-testid="preview-tab-image" />
              </div>
            </Match>
            <Match when={(kind() === 'markdown' || kind() === 'svg') && !source() && text() !== undefined}>
              <div class="ptab-md chat-prose md" data-testid="preview-tab-md">
                <For each={renderBlocks(text()!, false)}>{(html) => <MdBlock html={html} class="chat-block" />}</For>
              </div>
            </Match>
            <Match when={kind() === 'pdf' && bytes()}>
              {(b) => <PdfView url={b().url} name={name()} />}
            </Match>
            <Match when={kind() === 'audio' && bytes()}>
              {(b) => <audio controls src={b().url} class="media-audio" />}
            </Match>
            <Match when={kind() === 'video' && bytes()}>
              {(b) => <video controls src={b().url} class="media-video" />}
            </Match>
            <Match when={text() !== undefined}>
              <CodeBlock text={text()!} lang={languageOf(path()) ?? 'text'} />
            </Match>
            <Match when={kind() === 'other' || sniffedBinary()}>
              <EmptyState
                compact
                icon={<FileText size={22} />}
                title="No preview for this file type"
                body={name()}
                testId="preview-tab-nopreview"
                action={
                  <>
                    <button type="button" class="btn btn-secondary" onClick={editAsText}>Edit as text</button>
                    <Show
                      when={ownPc()}
                      fallback={<button type="button" class="btn btn-secondary" onClick={() => void download()}>Download</button>}
                    >
                      <OpenMenu path={path()} variant="button" onResult={flash} />
                    </Show>
                  </>
                }
              />
            </Match>
          </Switch>
        </Show>
      </div>
      <Show when={toast()}>
        <div class="media-toast" role="status">{toast()}</div>
      </Show>
    </div>
  );
}

setPaneComponent('preview', PreviewTab);
