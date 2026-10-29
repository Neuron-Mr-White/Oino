/**
 * Streaming block cache (UNI-113, shared in UNI-156): splits markdown into
 * top-level blocks (blank-line separated, fenced code/list items kept
 * whole) so a live stream only re-parses its last, still-growing block;
 * everything before it is rendered once and cached by source text.
 */
import { renderMarkdown, type RenderOptions } from './core';

/**
 * Splits markdown into top-level blocks (blank-line separated, fenced code
 * kept whole). An unclosed fence at the end stays one block (it's streaming).
 */
export function splitBlocks(text: string): string[] {
  const lines = text.split('\n');
  const blocks: string[] = [];
  let cur: string[] = [];
  let fence: string | null = null;
  let fenceInList = false;
  const push = () => {
    if (cur.some((l) => l.trim())) blocks.push(cur.join('\n'));
    cur = [];
  };
  const listItem = (l: string | undefined) => !!l && /^\s*([-*+]|\d+[.)])\s/.test(l);
  // Inside a list when the current block's last item line comes after any
  // non-list, non-indented line (a heading or paragraph may precede the list).
  const inList = () => {
    for (let k = cur.length - 1; k >= 0; k--) {
      const l = cur[k]!;
      if (!l.trim()) continue;
      if (listItem(l)) return true;
      if (/^\s{2,}\S/.test(l)) continue;
      return false;
    }
    return false;
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const m = /^\s{0,3}(```+|~~~+)/.exec(line);
    if (fence) {
      cur.push(line);
      if (m && m[1]![0] === fence[0] && m[1]!.length >= fence.length && line.trim() === m[1]) {
        fence = null;
        // A fence inside a list item doesn't end the list (more of the item may follow).
        if (!fenceInList) push();
      }
      continue;
    }
    if (m) {
      // A fence indented under a list item belongs to that item.
      fenceInList = /^\s{2,}/.test(line) && inList();
      if (!fenceInList) push();
      fence = m[1]!;
      cur.push(line);
      continue;
    }
    if (!line.trim()) {
      cur.push(line);
      // A blank line ends the block — unless a list continues after it
      // (next non-blank line is another item or indented).
      let j = i + 1;
      while (j < lines.length && !lines[j]!.trim()) j++;
      const next = lines[j];
      if (!(inList() && next !== undefined && (listItem(next) || /^\s{2,}\S/.test(next)))) push();
      continue;
    }
    cur.push(line);
  }
  push();
  return blocks;
}

const cache = new Map<string, string>();
const CACHE_MAX = 2000;

/** Renders blocks; all but the last come from the cache when unchanged. */
export function renderBlocks(text: string, live: boolean, opts: RenderOptions = {}): string[] {
  const blocks = splitBlocks(text);
  return blocks.map((block, i) => {
    const tail = live && i === blocks.length - 1;
    const key = opts.resolveAttachment ? undefined : block; // attachment resolution depends on external state — never cache across callers
    if (!tail && key !== undefined) {
      const hit = cache.get(key);
      if (hit !== undefined) return hit;
    }
    const html = renderMarkdown(block, opts);
    if (!tail && key !== undefined) {
      if (cache.size > CACHE_MAX) cache.clear();
      cache.set(key, html);
    }
    return html;
  });
}

/** Clears the render cache (tests). */
export function resetBlockCache(): void {
  cache.clear();
}
