/**
 * Mermaid diagrams (UNI-156): ```mermaid fences render lazily (mermaid is
 * only imported once a diagram actually appears), `securityLevel: 'strict'`,
 * dark theme from our tokens, the resulting SVG sanitised with DOMPurify's
 * svg profile. Cached by a hash of the source so re-renders (streaming,
 * re-mount) are free. Call sites render the closed fence only — the
 * streaming tail stays a plain code block until its own fence closes.
 */
import DOMPurify from 'dompurify';

let mermaidPromise: Promise<typeof import('mermaid').default> | undefined;
async function loadMermaid() {
  if (!mermaidPromise) {
    mermaidPromise = import('mermaid').then((m) => {
      m.default.initialize({
        startOnLoad: false,
        securityLevel: 'strict',
        // Errors are ours to show (code + note), never mermaid's bomb graphic.
        suppressErrorRendering: true,
        theme: 'dark',
        themeVariables: {
          background: '#141211',
          primaryColor: '#26211F',
          primaryTextColor: '#F5EFE6',
          primaryBorderColor: '#2E2825',
          lineColor: '#A39A90',
          secondaryColor: '#1C1917',
          tertiaryColor: '#0A0908',
          fontFamily: 'Inter Variable, sans-serif',
        },
      });
      return m.default;
    });
  }
  return mermaidPromise;
}

// djb2: fast, good enough to key a render cache (not cryptographic).
function hash(source: string): string {
  let h = 5381;
  for (let i = 0; i < source.length; i++) h = (h * 33) ^ source.charCodeAt(i);
  return (h >>> 0).toString(36);
}

export type MermaidResult = { ok: true; svg: string } | { ok: false; error: string };

/** Render cache, LRU-bounded: a live graph re-renders on every count change. */
const CACHE_MAX = 48;
const cache = new Map<string, MermaidResult>();
const remember = (key: string, result: MermaidResult) => {
  cache.delete(key);
  cache.set(key, result);
  while (cache.size > CACHE_MAX) cache.delete(cache.keys().next().value as string);
};
let seq = 0;

/** Renders mermaid source to a sanitised `<svg>` string, or an error. Cached by source hash. */
export async function renderMermaid(source: string): Promise<MermaidResult> {
  const key = hash(source);
  const hit = cache.get(key);
  if (hit) {
    remember(key, hit);
    return hit;
  }
  const id = `mmd-${Date.now().toString(36)}-${seq++}`;
  try {
    const mermaid = await loadMermaid();
    // Validate first: a bad diagram never reaches render (which can leave
    // its error SVG / temp container in <body>).
    await mermaid.parse(source);
    const { svg } = await mermaid.render(id, source);
    // Mermaid labels text through `foreignObject > div > span` even under
    // `securityLevel: 'strict'`. DOMPurify's svg profile forbids
    // `foreignObject`'s *contents* outright (DEFAULT_FORBID_CONTENTS) and
    // doesn't know it's a valid HTML integration point (only MathML's
    // `annotation-xml` is, by default) — so labels render as empty boxes
    // unless we explicitly allow both. Scripts/event handlers still don't
    // survive (plain tag/attribute allow-lists, same as the rest of DOMPurify).
    const clean = DOMPurify.sanitize(svg, {
      USE_PROFILES: { svg: true, svgFilters: true, html: true },
      ADD_TAGS: ['foreignObject'],
      FORBID_CONTENTS: [],
      HTML_INTEGRATION_POINTS: { foreignobject: true },
    });
    const result: MermaidResult = { ok: true, svg: clean };
    remember(key, result);
    return result;
  } catch (error) {
    // Mermaid renders into a temp element in <body> (`d<id>` wrapper and
    // the svg itself); drop leftovers so a failed diagram never sticks to
    // the bottom of the page.
    if (typeof document !== 'undefined') for (const el of [document.getElementById(id), document.getElementById(`d${id}`)]) el?.remove();
    const result: MermaidResult = { ok: false, error: error instanceof Error ? error.message : String(error) };
    remember(key, result);
    return result;
  }
}

/** Clears the render cache (tests). */
export function resetMermaidCache(): void {
  cache.clear();
}

/** Recognised mermaid fence language. */
export function isMermaidLang(lang: string): boolean {
  return lang.trim().toLowerCase() === 'mermaid';
}
