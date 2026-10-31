/**
 * Shared markdown renderer (UNI-156): marked + DOMPurify, used by chat
 * prose/thinking/cards and board bodies/reports/summaries alike.
 *
 *  - `breaks: true` — a single newline is a line break (pi's TUI).
 *  - Code blocks get a sticky header (language + Copy). Closed fences are
 *    highlighted lazily (lib/markdown/highlight.ts, CodeMirror/Lezer
 *    grammars loaded on demand) — `hydrateCode` swaps the plain escaped
 *    text for highlighted spans once the grammar (or diagram) is ready;
 *    the synchronous render always has a valid plain fallback.
 *  - ```diff/```patch fences render as a full unified-diff view
 *    synchronously (lib/markdown/diff.ts).
 *  - ```mermaid fences render as a "Diagram…" placeholder; `hydrateCode`
 *    swaps in the sanitised SVG once mermaid (lazy chunk) renders it.
 *  - `attachments` option turns `att:<task>/<name>` links/images into
 *    chips/media (lib/markdown/attachments.ts); callers resolve the path
 *    and later call `hydrateAttachments`.
 */
import { Marked, type Tokens } from 'marked';
import DOMPurify from 'dompurify';
import { escapeHtml } from './escape';
import { attachmentHtml, attachmentLinkHtml } from './attachments';
import { isDiffLang, parseUnifiedDiff, renderDiffHtml } from './diff';
import { isMermaidLang } from './mermaid';
import { canonicalLang, highlightHtml, isSupportedLang } from './highlight';

export type RenderOptions = {
  /** Resolves `att:` references to a fetchable path; omit to leave them as plain links/text. */
  resolveAttachment?: (reference: string) => string | null;
  /** Render `<img>` for plain (non-`att:`) image markdown. Off by default — chat prose never embeds arbitrary images. */
  allowImages?: boolean;
};

const LONG_BLOCK_LINES = 20;

function codeBlockHtml(lang: string, text: string): string {
  const label = lang || 'text';
  const highlighted = isSupportedLang(label);
  const lineCount = text.split('\n').length;
  const long = lineCount > LONG_BLOCK_LINES;
  const wrapToggle = long ? `<button type="button" class="cb-wrap-toggle" data-wrap-toggle>Wrap</button>` : '';
  const body = long ? wrapLines(escapeHtml(text)) : escapeHtml(text);
  return (
    `<div class="cb${long ? ' line-numbers' : ''}"><div class="cb-head"><span class="cb-lang">${escapeHtml(label)}</span>` +
    `<span class="cb-head-actions">${wrapToggle}<button type="button" class="cb-copy" data-copy>Copy</button></span></div>` +
    `<pre><code${highlighted ? ` data-cb-lang="${escapeHtml(canonicalLang(label))}"` : ''}>${body}</code></pre></div>`
  );
}

/** Wraps highlighted/plain code text in one `<span class="cb-line">` per line, for the CSS line-number counter. */
function wrapLines(html: string): string {
  return html
    .split('\n')
    .map((line) => `<span class="cb-line">${line}</span>`)
    .join('\n');
}

function mermaidBlockHtml(text: string, closed: boolean): string {
  const source = encodeURIComponent(text);
  if (!closed) return `<pre class="cb mmd-plain"><code>${escapeHtml(text)}</code></pre>`;
  return (
    `<div class="mmd" data-mmd-source="${source}">` +
    `<div class="mmd-pending"><span class="mmd-label">Diagram…</span><pre><code>${escapeHtml(text)}</code></pre></div>` +
    `</div>`
  );
}

function tableHtml(token: Tokens.Table, parser: { parseInline(tokens: Tokens.Text['tokens']): string }): string {
  const aligns = token.align;
  const head = token.header
    .map((c, i) => `<th${aligns[i] ? ` class="al-${aligns[i]}"` : ''}>${parser.parseInline(c.tokens)}</th>`)
    .join('');
  const body = token.rows
    .map((r) => `<tr>${r.map((c, i) => `<td${aligns[i] ? ` class="al-${aligns[i]}"` : ''}>${parser.parseInline(c.tokens)}</td>`).join('')}</tr>`)
    .join('');
  return `<div class="tbl"><table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>`;
}

function buildMarked(opts: RenderOptions): Marked {
  const md = new Marked({ gfm: true, breaks: true });
  md.use({
    renderer: {
      code(token: Tokens.Code) {
        const lang = (token.lang ?? '').trim().split(/\s+/)[0] ?? '';
        if (isDiffLang(lang)) {
          return (
            `<div class="cb"><div class="cb-head"><span class="cb-lang">${escapeHtml(lang)}</span>` +
            `<span class="cb-head-actions"><button type="button" class="cb-copy" data-copy>Copy</button></span></div>` +
            `${renderDiffHtml(parseUnifiedDiff(token.text))}` +
            `<span hidden data-diff-raw>${escapeHtml(token.text)}</span></div>`
          );
        }
        if (isMermaidLang(lang)) return mermaidBlockHtml(token.text, true);
        return codeBlockHtml(lang, token.text);
      },
      table(token: Tokens.Table) {
        return tableHtml(token, this.parser);
      },
      image(token: Tokens.Image) {
        const att = opts.resolveAttachment ? attachmentHtml(token.href, token.text, opts.resolveAttachment) : null;
        if (att) return att;
        return `<img src="${escapeHtml(token.href)}" alt="${escapeHtml(token.text)}"${token.title ? ` title="${escapeHtml(token.title)}"` : ''}>`;
      },
      link(token: Tokens.Link) {
        const text = this.parser.parseInline(token.tokens);
        const att = opts.resolveAttachment ? attachmentLinkHtml(token.href, text.replace(/<[^>]+>/g, ''), opts.resolveAttachment) : null;
        if (att) return att;
        return `<a href="${escapeHtml(token.href)}"${token.title ? ` title="${escapeHtml(token.title)}"` : ''}>${text}</a>`;
      },
    },
  });
  return md;
}

const chatMd = buildMarked({});

/** Renders chat/card/thinking markdown (no attachments, images stripped unless `allowImages`). */
export function renderMarkdown(text: string | undefined, opts: RenderOptions = {}): string {
  const source = (text ?? '').trim();
  if (!source) return '';
  const md = opts.resolveAttachment ? buildMarked(opts) : chatMd;
  const html = md.parse(source, { async: false }) as string;
  return sanitize(html, opts);
}

// Task-list checkboxes (`<input type=checkbox disabled>`, GFM) are the only
// <input> OUR markdown ever emits; raw HTML in the source could still try
// to smuggle other input types in, so strip anything that isn't exactly
// that shape (belt-and-braces beyond ADD_ATTR's allow-list below).
let inputHookAdded = false;
function ensureInputHook(): void {
  if (inputHookAdded) return;
  inputHookAdded = true;
  DOMPurify.addHook('afterSanitizeAttributes', (node) => {
    if (node.nodeName !== 'INPUT') return;
    const el = node as HTMLInputElement;
    if (el.getAttribute('type') !== 'checkbox' || !el.hasAttribute('disabled')) el.remove();
  });
}

function sanitize(html: string, opts: RenderOptions): string {
  ensureInputHook();
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    ADD_TAGS: ['video', 'audio', 'mark', 'input'],
    ADD_ATTR: [
      'data-copy',
      'data-wrap-toggle',
      'data-diff-raw',
      'hidden',
      'target',
      'controls',
      'preload',
      'loading',
      'data-att-path',
      'data-att-name',
      'data-cb-id',
      'data-cb-lang',
      'data-mmd-source',
      'type',
      'checked',
      'disabled',
    ],
    FORBID_TAGS: opts.allowImages || opts.resolveAttachment ? ['script', 'style', 'form', 'iframe'] : ['script', 'style', 'form', 'iframe', 'img'],
    FORBID_ATTR: ['onerror', 'onload', 'onclick'],
    // DOMPurify checks this against every URI-*looking* attribute value,
    // including unrelated ones like `<input type=checkbox>` — so, like its
    // own default, it must also accept plain non-URL tokens (no scheme, no
    // colon), not just the schemes we allow.
    ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto):|#|data:image\/|[^a-z]|[a-z+.-]+(?:[^a-z+.\-:]|$))/i,
  });
}

/**
 * Hydrates one rendered container: swaps plain code for highlighted spans
 * (lazy grammar) and mermaid placeholders for sanitised SVGs (lazy
 * mermaid). Safe to call repeatedly; already-hydrated nodes are skipped.
 */
export async function hydrateCode(root: ParentNode): Promise<void> {
  const codeNodes = [...root.querySelectorAll<HTMLElement>('code[data-cb-lang]:not([data-hl-done])')];
  const mmdNodes = [...root.querySelectorAll<HTMLElement>('.mmd[data-mmd-source]:not([data-mmd-done])')];
  await Promise.all([
    ...codeNodes.map(async (node) => {
      const lang = node.dataset.cbLang ?? '';
      // Un-wrap cb-line spans (if any) to recover the raw source for highlighting.
      const source = (node.textContent ?? '').replace(/\n$/, '');
      const html = await highlightHtml(source, lang);
      if (html !== undefined) {
        const long = node.closest('.cb')?.classList.contains('line-numbers');
        node.innerHTML = long ? wrapLines(html) : html;
        node.dataset.hlDone = '1';
      }
    }),
    ...mmdNodes.map(async (node) => {
      const source = decodeURIComponent(node.dataset.mmdSource ?? '');
      node.dataset.mmdDone = '1';
      const { renderMermaid } = await import('./mermaid');
      const result = await renderMermaid(source);
      if (result.ok) {
        node.innerHTML = `<div class="mmd-svg" data-mmd-tap>${result.svg}</div>`;
      } else {
        node.innerHTML =
          `<pre class="cb mmd-plain"><code>${escapeHtml(source)}</code></pre>` +
          `<p class="mmd-error">Couldn't draw this diagram</p>`;
      }
    }),
  ]);
}

export { escapeHtml };
