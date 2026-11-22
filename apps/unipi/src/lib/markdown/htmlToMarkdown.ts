/**
 * Rendered-markdown DOM → markdown source (copy-as-markdown). A small,
 * focused converter for exactly what `renderMarkdown` (core.ts) emits —
 * headings, emphasis, inline/fenced code, lists (nested, ordered, task),
 * links, images, blockquotes, GFM tables, hr, mermaid/diff fences — and
 * tolerant of the partial fragments `Range.cloneContents()` produces.
 *
 * UI chrome (code-block header + Copy/Wrap buttons, path-copy buttons,
 * "Stopped" badge, KaTeX's hidden MathML) never leaks into the output.
 */

/** Elements that are chrome, not content. */
const CHROME =
  'button, script, style, .cb-head, .code-copy, .chat-stopped, .mmd-label, .mmd-error, .katex-mathml, [data-copy], [data-wrap-toggle], [data-copy-path]';

const BLOCK = new Set([
  'P', 'DIV', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'UL', 'OL', 'LI', 'BLOCKQUOTE', 'PRE', 'TABLE', 'THEAD', 'TBODY', 'TFOOT', 'TR',
  'HR', 'SECTION', 'ARTICLE', 'DETAILS', 'SUMMARY', 'FIGURE', 'FIGCAPTION',
]);

/** True for nodes the converter drops (chrome, hidden helpers, comments). */
export function isChrome(node: Node): boolean {
  if (node.nodeType !== 1) return node.nodeType !== 3;
  const el = node as Element;
  if (el.hasAttribute('hidden') && !el.hasAttribute('data-diff-raw')) return true;
  return el.matches(CHROME);
}

/** Converts a rendered fragment/element to markdown. */
export function htmlToMarkdown(root: Node): string {
  return blocks(root).join('\n\n').trim();
}

function blocks(parent: Node): string[] {
  const out: string[] = [];
  let buf = '';
  let prevBr = true;
  const flush = () => {
    const t = buf.replace(/^[ \t\n]+|[ \t\n]+$/g, '');
    if (t) out.push(t);
    buf = '';
    prevBr = true;
  };
  let skip = 0;
  for (const child of Array.from(parent.childNodes)) {
    if (skip > 0) {
      skip--;
      continue;
    }
    if (isChrome(child)) continue;
    if (child.nodeType === 1 && (child as Element).tagName === 'LI') {
      // Orphan items (a Range clone of a list drops the <ul>) group into one list.
      flush();
      const run = [child as Element];
      let next = child.nextSibling;
      while (next && next.nodeType === 1 && (next as Element).tagName === 'LI') {
        run.push(next as Element);
        next = next.nextSibling;
      }
      out.push(listItems(run, false, 1));
      skip = run.length - 1;
    } else if (child.nodeType === 1 && BLOCK.has((child as Element).tagName)) {
      flush();
      out.push(...block(child as Element));
    } else {
      buf += inline(child, prevBr);
      prevBr = child.nodeType === 1 && (child as Element).tagName === 'BR';
    }
  }
  flush();
  return out;
}

function block(el: Element): string[] {
  const tag = el.tagName;
  if (tag === 'HR') return ['---'];
  if (/^H[1-6]$/.test(tag)) {
    const text = inlineChildren(el).trim();
    return text ? [`${'#'.repeat(Number(tag[1]))} ${text}`] : [];
  }
  if (tag === 'P') {
    const text = inlineChildren(el).trim();
    return text ? [text] : [];
  }
  if (tag === 'PRE') return [codeFence(el)];
  if (tag === 'UL' || tag === 'OL') return [list(el)];
  if (tag === 'LI') return [listItems([el], false, 1)];
  if (tag === 'BLOCKQUOTE') {
    const inner = blocks(el).join('\n\n');
    return inner ? [inner.split('\n').map((l) => (l ? `> ${l}` : '>')).join('\n')] : [];
  }
  if (tag === 'TABLE' || tag === 'THEAD' || tag === 'TBODY' || tag === 'TFOOT' || tag === 'TR') {
    const t = table(el);
    return t ? [t] : [];
  }
  if (tag === 'DIV') {
    const mmd = el.getAttribute('data-mmd-source');
    if (el.classList.contains('mmd') && mmd !== null) return [fence(safeDecode(mmd), 'mermaid')];
    const raw = el.querySelector('[data-diff-raw]');
    if (raw && el.classList.contains('cb')) return [fence(raw.textContent ?? '', langLabel(el) || 'diff')];
  }
  return blocks(el);
}

/* ── inline ───────────────────────────────────────────────────────── */

function inlineChildren(parent: Node, atStart = true): string {
  let out = '';
  let prevBr = atStart;
  for (const child of Array.from(parent.childNodes)) {
    if (isChrome(child)) continue;
    out += inline(child, prevBr);
    prevBr = child.nodeType === 1 && (child as Element).tagName === 'BR';
  }
  return out;
}

function escapeText(text: string): string {
  return text
    .replace(/`/g, '\\`')
    .replace(/\*/g, (m, i: number, s: string) => (/\s/.test(s[i - 1] ?? ' ') && /\s/.test(s[i + 1] ?? ' ') ? m : '\\*'))
    .replace(/_/g, (m, i: number, s: string) => (/[A-Za-z0-9]/.test(s[i - 1] ?? '') && /[A-Za-z0-9]/.test(s[i + 1] ?? '') ? m : '\\_'));
}

/** Wraps `inner` in `mark`, keeping edge whitespace outside the markers. */
function wrap(inner: string, mark: string): string {
  const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(inner)!;
  return m[2] ? `${m[1]}${mark}${m[2]}${mark}${m[3]}` : inner;
}

function inlineCode(text: string): string {
  const runs = text.match(/`+/g) ?? [];
  const ticks = '`'.repeat(Math.max(0, ...runs.map((r) => r.length)) + (runs.length ? 1 : 0) || 1);
  const pad = /^`|`$/.test(text) ? ' ' : '';
  return `${ticks}${pad}${text}${pad}${ticks}`;
}

function linkTarget(href: string): string {
  return /[\s()]/.test(href) ? `<${href}>` : href;
}

function inline(node: Node, atLineStart: boolean): string {
  if (node.nodeType === 3) {
    let t = (node.textContent ?? '').replace(/[ \t\n\r]+/g, ' ');
    if (atLineStart) t = t.replace(/^ /, '');
    return escapeText(t);
  }
  if (node.nodeType !== 1) return '';
  const el = node as Element;
  switch (el.tagName) {
    case 'BR':
      return '  \n';
    case 'STRONG':
    case 'B':
      return wrap(inlineChildren(el, false), '**');
    case 'EM':
    case 'I':
      return wrap(inlineChildren(el, false), '_');
    case 'DEL':
    case 'S':
    case 'STRIKE':
      return wrap(inlineChildren(el, false), '~~');
    case 'CODE':
      return inlineCode(el.textContent ?? '');
    case 'A': {
      const inner = inlineChildren(el, false);
      const href = el.getAttribute('href');
      if (!href) return inner;
      if (inner.trim() === href) return href;
      const title = el.getAttribute('title');
      return `[${inner}](${linkTarget(href)}${title ? ` "${title.replace(/"/g, '\\"')}"` : ''})`;
    }
    case 'IMG': {
      const src = el.getAttribute('src') ?? '';
      const title = el.getAttribute('title');
      return `![${el.getAttribute('alt') ?? ''}](${linkTarget(src)}${title ? ` "${title.replace(/"/g, '\\"')}"` : ''})`;
    }
    case 'INPUT':
      return el.getAttribute('type') === 'checkbox' ? (el.hasAttribute('checked') || (el as HTMLInputElement).checked ? '[x]' : '[ ]') : '';
    default: {
      const tex = el.getAttribute('data-tex');
      if (tex !== null) return el.hasAttribute('data-display') || el.classList.contains('katex-display') ? `$$${tex}$$` : `$${tex}$`;
      return inlineChildren(el, false);
    }
  }
}

/* ── code ─────────────────────────────────────────────────────────── */

function fence(code: string, lang: string): string {
  const runs = code.match(/`{3,}/g) ?? [];
  const f = '`'.repeat(Math.max(3, ...runs.map((r) => r.length + 1)));
  return `${f}${lang}\n${code}\n${f}`;
}

/** The language label of a `.cb` block (`text` is the renderer's no-language placeholder). */
function langLabel(cb: Element | null): string {
  const label = cb?.querySelector('.cb-lang')?.textContent?.trim() ?? '';
  return label === 'text' ? '' : label;
}

function codeFence(pre: Element): string {
  const code = pre.querySelector('code') ?? pre;
  const text = (code.textContent ?? '').replace(/\n$/, '');
  const lang =
    langLabel(pre.closest('.cb')) || code.getAttribute('data-cb-lang') || /(?:^|\s)language-([\w+#-]+)/.exec(code.getAttribute('class') ?? '')?.[1] || '';
  return fence(text, lang);
}

function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/* ── lists ────────────────────────────────────────────────────────── */

function list(el: Element): string {
  const start = el.tagName === 'OL' ? parseInt(el.getAttribute('start') ?? '1', 10) || 1 : 1;
  return listItems(Array.from(el.children).filter((c) => c.tagName === 'LI'), el.tagName === 'OL', start);
}

function listItems(items: Element[], ordered: boolean, start: number): string {
  // marked renders a loose list's items as <p>…</p>; a tight list's as bare text.
  const loose = items.some((li) => Array.from(li.children).some((c) => c.tagName === 'P'));
  const rendered = items.map((li, i) => {
    const marker = ordered ? `${start + i}. ` : '- ';
    const body = blocks(li).join(loose ? '\n\n' : '\n');
    if (!body) return marker.trimEnd();
    const pad = ' '.repeat(marker.length);
    return marker + body.split('\n').map((l, j) => (j === 0 || !l ? l : pad + l)).join('\n');
  });
  return rendered.join(loose ? '\n\n' : '\n');
}

/* ── tables ───────────────────────────────────────────────────────── */

function alignOf(cell: Element): '' | 'left' | 'center' | 'right' {
  const m =
    /(?:^|\s)al-(left|center|right)\b/.exec(cell.getAttribute('class') ?? '') ??
    /text-align:\s*(left|center|right)/.exec(cell.getAttribute('style') ?? '') ??
    /^(left|center|right)$/.exec(cell.getAttribute('align') ?? '');
  return (m?.[1] as 'left' | 'center' | 'right' | undefined) ?? '';
}

function cellText(cell: Element): string {
  return inlineChildren(cell)
    .trim()
    .replace(/ *\n */g, '<br>')
    .replace(/(^|[^\\])\|/g, '$1\\|');
}

function table(el: Element): string {
  const rows = Array.from(el.tagName === 'TR' ? [el] : el.querySelectorAll('tr')).map((r) =>
    Array.from(r.children).filter((c) => c.tagName === 'TH' || c.tagName === 'TD'),
  );
  if (!rows.length) return '';
  const cols = Math.max(...rows.map((r) => r.length));
  const text = rows.map((r) => Array.from({ length: cols }, (_, i) => (r[i] ? cellText(r[i]!) : '')));
  const line = (cells: string[]) => `| ${cells.join(' | ')} |`;
  const delim = Array.from({ length: cols }, (_, i) => {
    const a = rows[0]![i] ? alignOf(rows[0]![i]!) : '';
    return a === 'left' ? ':---' : a === 'center' ? ':---:' : a === 'right' ? '---:' : '---';
  });
  return [line(text[0]!), line(delim), ...text.slice(1).map(line)].join('\n');
}
