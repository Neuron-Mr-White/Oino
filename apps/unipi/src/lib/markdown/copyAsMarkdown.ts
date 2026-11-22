/**
 * Copy-as-markdown: copying a selection from rendered markdown puts the
 * ORIGINAL MARKDOWN on the clipboard (text/plain) plus the selection's
 * HTML (text/html), instead of the browser's flattened plain text.
 *
 * One delegated `copy` listener on the document (installed lazily by the
 * renderer, see core.ts) — desktop Ctrl+C, the context menu and phone
 * long-press → Copy all fire it. A selection is handled when it lies
 * entirely inside ONE rendered-markdown root (the outermost `.md`):
 *   - inside a single code block → the raw code, no fences;
 *   - the whole root, and the root registered its source
 *     (`registerMarkdownSource`) → that exact source;
 *   - otherwise → the cloned selection fragment through `htmlToMarkdown`.
 * Anything else (selections leaving the root, inputs/textareas) keeps the
 * browser's default copy.
 */
import { htmlToMarkdown, isChrome } from './htmlToMarkdown';

const sources = new WeakMap<Element, () => string>();

/** Declares the exact markdown source behind a rendered root, so a whole-message copy is lossless. */
export function registerMarkdownSource(root: Element, source: () => string): () => void {
  sources.set(root, source);
  return () => {
    if (sources.get(root) === source) sources.delete(root);
  };
}

export type SelectionCopy = { markdown: string; html: string };

const INLINE_WRAP = new Set(['STRONG', 'B', 'EM', 'I', 'DEL', 'S', 'STRIKE', 'A', 'MARK']);
const LIST_WRAP = new Set(['UL', 'OL', 'BLOCKQUOTE']);
const TABLE_WRAP = new Set(['TABLE', 'THEAD', 'TBODY', 'TFOOT', 'TR']);
/** A root must contain something markdown-shaped (the `.md` class also wraps diff views). */
const MARKDOWN_SHAPE = 'p, li, h1, h2, h3, h4, h5, h6, pre, table, blockquote, strong, em, code, a, hr, img';

const elementOf = (node: Node): Element | null => (node.nodeType === 1 ? (node as Element) : node.parentElement);

/** The outermost `.md` containing the whole range, or null when the range leaves one root. */
export function markdownRootOf(range: Range): Element | null {
  let root = elementOf(range.commonAncestorContainer)?.closest('.md') ?? null;
  if (!root) return null;
  for (let outer = root.parentElement?.closest('.md'); outer; outer = outer.parentElement?.closest('.md')) root = outer;
  return root.querySelector(MARKDOWN_SHAPE) ? root : null;
}

/** Text with whitespace removed and chrome dropped — a robust "same content?" key. */
function contentKey(node: Node): string {
  let out = '';
  for (const child of Array.from(node.childNodes)) {
    if (isChrome(child)) continue;
    out += child.nodeType === 3 ? child.textContent : contentKey(child);
  }
  return out.replace(/\s+/g, '');
}

/** The child of `parent` that contains `node` (or `node` itself). */
function childOf(parent: Element, node: Node): Node | null {
  let cur: Node | null = node;
  while (cur && cur.parentNode !== parent) cur = cur.parentNode;
  return cur;
}

/**
 * Re-creates the structural ancestors `cloneContents()` drops, so a
 * selection of `<li>`s / `<tr>`s / bold text still converts as a list /
 * table / bold run. Block wrappers (p, h2, li of a single item) are left
 * off on purpose: selecting a few words of a heading copies words, not `## words`.
 */
function wrapFragment(doc: Document, range: Range, root: Element): Element {
  let inner: Node = range.cloneContents();
  let anc = elementOf(range.commonAncestorContainer);
  let sawContainer = false;
  while (anc && anc !== root && root.contains(anc)) {
    const tag = anc.tagName;
    const inlineOk = INLINE_WRAP.has(tag) && !sawContainer;
    const containerOk = LIST_WRAP.has(tag) || TABLE_WRAP.has(tag);
    if (!inlineOk && !containerOk) break;
    const w = anc.cloneNode(false) as Element;
    if (tag === 'OL') {
      const from = childOf(anc, range.startContainer);
      const items = Array.from(anc.children).filter((c) => c.tagName === 'LI');
      const idx = from ? Math.max(0, items.indexOf(from as Element)) : (range.startContainer === anc ? range.startOffset : 0);
      w.setAttribute('start', String((parseInt(anc.getAttribute('start') ?? '1', 10) || 1) + idx));
    }
    w.appendChild(inner);
    inner = w;
    if (containerOk) {
      sawContainer = true;
      if (!TABLE_WRAP.has(tag) || tag === 'TABLE') break;
    }
    anc = anc.parentElement;
  }
  const holder = doc.createElement('div');
  holder.appendChild(inner);
  return holder;
}

function stripChrome(el: Element): void {
  for (const node of Array.from(el.querySelectorAll('*'))) if (isChrome(node)) node.remove();
}

const escapeHtml = (s: string): string => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** What a copy of `range` (inside `root`) should put on the clipboard, or null to keep the default. */
export function selectionToClipboard(range: Range, root: Element): SelectionCopy | null {
  const doc = root.ownerDocument;
  // One code block → the raw code.
  const pre = elementOf(range.startContainer)?.closest('pre') ?? null;
  if (pre && root.contains(pre) && pre === elementOf(range.endContainer)?.closest('pre')) {
    const raw = range.toString();
    return raw ? { markdown: raw, html: `<pre><code>${escapeHtml(raw)}</code></pre>` } : null;
  }
  const holder = wrapFragment(doc, range, root);
  const html = (() => {
    const copy = holder.cloneNode(true) as Element;
    stripChrome(copy);
    return copy.innerHTML;
  })();
  // The whole message → its exact source.
  const source = sources.get(root)?.();
  if (source !== undefined && contentKey(holder) === contentKey(root)) return { markdown: source, html };
  const markdown = htmlToMarkdown(holder);
  return markdown ? { markdown, html } : null;
}

type CopyEventLike = Event & { clipboardData?: DataTransfer | null };

function onCopy(event: Event): void {
  const e = event as CopyEventLike;
  const doc = (e.target as Node | null)?.ownerDocument ?? document;
  const data = e.clipboardData;
  const sel = doc.getSelection();
  if (!data || !sel || sel.isCollapsed || sel.rangeCount !== 1) return;
  const active = doc.activeElement as HTMLElement | null;
  if (active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA' || active.isContentEditable)) return;
  try {
    const range = sel.getRangeAt(0);
    const root = markdownRootOf(range);
    if (!root) return;
    const out = selectionToClipboard(range, root);
    if (!out) return;
    data.setData('text/plain', out.markdown);
    data.setData('text/html', out.html);
    e.preventDefault();
  } catch {
    // Never break copy: fall back to the browser's own.
  }
}

const installed = new WeakSet<Document>();

/** Installs the delegated copy listener once per document. */
export function installCopyAsMarkdown(doc: Document = document): void {
  if (installed.has(doc)) return;
  installed.add(doc);
  doc.addEventListener('copy', onCopy);
}
