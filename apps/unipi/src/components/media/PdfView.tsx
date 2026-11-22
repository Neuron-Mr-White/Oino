/**
 * PDF viewer (UNI-197). Android's WebView has no built-in PDF renderer, so
 * an <iframe src=blob:…pdf> painted a blank white page. pdf.js renders
 * each page to a canvas instead; pages render lazily as they scroll in.
 */
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import type { PDFDocumentProxy } from 'pdfjs-dist';

// The legacy build: the modern one calls Uint8Array#toHex (Chrome 140+);
// phones run older WebViews (Redmi: 136 → "r.toHex is not a function").

let lib: Promise<typeof import('pdfjs-dist/legacy/build/pdf.mjs')> | undefined;
function pdfjs() {
  lib ??= (async () => {
    const mod = await import('pdfjs-dist/legacy/build/pdf.mjs');
    // Vite bundles the worker as a .js chunk: Tauri's asset server types
    // .mjs loosely, and a module worker needs a JS MIME type.
    mod.GlobalWorkerOptions.workerPort = new Worker(new URL('pdfjs-dist/legacy/build/pdf.worker.min.mjs', import.meta.url), { type: 'module' });
    return mod;
  })();
  return lib;
}

export function PdfView(props: { url: string; name?: string }) {
  const [doc, setDoc] = createSignal<PDFDocumentProxy | undefined>();
  const [error, setError] = createSignal<string | undefined>();
  let host: HTMLDivElement | undefined;

  onMount(() => {
    let cancelled = false;
    let loaded: PDFDocumentProxy | undefined;
    void (async () => {
      try {
        const mod = await pdfjs();
        const data = new Uint8Array(await (await fetch(props.url)).arrayBuffer());
        loaded = await mod.getDocument({ data }).promise;
        if (cancelled) void loaded.destroy();
        else setDoc(loaded);
      } catch (e) {
        if (!cancelled) setError((e as { message?: string })?.message ?? "Couldn't open this PDF");
      }
    })();
    onCleanup(() => {
      cancelled = true;
      void loaded?.destroy();
    });
  });

  return (
    <div class="pdf-view" ref={host} data-testid="pdf-view">
      <Show when={!error()} fallback={<p class="text-text-muted text-sm p-4">{error()} — share it to open with another app.</p>}>
        <Show when={doc()} fallback={<p class="text-text-muted text-sm p-4" role="status">Loading PDF…</p>}>
          {(d) => (
            <>
              <p class="pdf-count">{d().numPages} {d().numPages === 1 ? 'page' : 'pages'}</p>
              <For each={Array.from({ length: d().numPages }, (_, i) => i + 1)}>{(n) => <PdfPage doc={d()} number={n} root={host} />}</For>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}

function PdfPage(props: { doc: PDFDocumentProxy; number: number; root?: HTMLElement }) {
  let canvas: HTMLCanvasElement | undefined;
  let box: HTMLDivElement | undefined;
  const [ratio, setRatio] = createSignal(1.414);
  let rendered = false;

  const render = async () => {
    if (rendered || !canvas || !box) return;
    rendered = true;
    const page = await props.doc.getPage(props.number);
    const base = page.getViewport({ scale: 1 });
    setRatio(base.height / base.width);
    const width = box.clientWidth || window.innerWidth;
    const dpr = Math.min(window.devicePixelRatio || 1, 2.5);
    const viewport = page.getViewport({ scale: (width / base.width) * dpr });
    canvas.width = Math.floor(viewport.width);
    canvas.height = Math.floor(viewport.height);
    await page.render({ canvas, viewport }).promise;
  };

  onMount(() => {
    // Size the placeholder from page 1's ratio right away; render when near.
    if (typeof IntersectionObserver === 'undefined') {
      void render();
      return;
    }
    const io = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && void render(), { rootMargin: '600px 0px' });
    io.observe(box!);
    onCleanup(() => io.disconnect());
  });

  return (
    <div class="pdf-page" ref={box} style={{ 'aspect-ratio': `1 / ${ratio()}` }} aria-label={`Page ${props.number}`}>
      <canvas ref={canvas} />
    </div>
  );
}
