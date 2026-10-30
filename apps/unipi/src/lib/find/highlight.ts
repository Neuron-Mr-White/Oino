/**
 * Find highlighting over the virtual timeline (WP-A18, Z-79): DOM-level, so
 * the static-once-complete markdown (Z-34) and the virtualizer stay as they
 * are. `applyFindHighlights` walks the text nodes of the timeline content
 * (user bubbles, answers, live tail — whatever is mounted) and wraps each
 * case-insensitive hit in `<mark class="find-hit">`; the active match gets
 * `data-active` and scrolls into view. `clearFindHighlights` undoes it.
 */

const MARK = 'find-hit';

/** Removes every highlight under `root`, restoring plain text nodes. */
export function clearFindHighlights(root: ParentNode): void {
  for (const mark of [...root.querySelectorAll<HTMLElement>(`mark.${MARK}`)]) {
    const parent = mark.parentNode;
    if (!parent) continue;
    while (mark.firstChild) parent.insertBefore(mark.firstChild, mark);
    mark.remove();
    parent.normalize();
  }
}

/**
 * Highlights every occurrence of `query` under `root` (skipping existing
 * marks, scripts and styles). Returns how many were wrapped.
 */
export function applyFindHighlights(root: ParentNode, query: string): number {
  const needle = query.toLowerCase();
  if (!needle) return 0;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      const el = node.parentElement;
      if (!el) return NodeFilter.FILTER_REJECT;
      if (el.closest('mark.find-hit, script, style')) return NodeFilter.FILTER_REJECT;
      return node.nodeValue && node.nodeValue.toLowerCase().includes(needle) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    },
  });
  const hits: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) hits.push(n as Text);
  let count = 0;
  for (const node of hits) count += wrapMatches(node, needle);
  return count;
}

function wrapMatches(node: Text, needle: string): number {
  let count = 0;
  let at = node.nodeValue?.toLowerCase().indexOf(needle) ?? -1;
  while (at >= 0 && node.nodeValue) {
    const mark = document.createElement('mark');
    mark.className = MARK;
    const hit = node.splitText(at);
    const rest = hit.splitText(needle.length);
    mark.appendChild(hit);
    node.parentNode?.insertBefore(mark, rest);
    count += 1;
    node = rest;
    at = rest.nodeValue?.toLowerCase().indexOf(needle) ?? -1;
  }
  return count;
}

/**
 * Marks the `occurrence`-th highlight inside the turn's row (`data-turn`)
 * active and centres it. Returns false when it isn't mounted (the caller
 * re-runs after the virtualizer mounts the turn).
 */
export function activateFindMark(root: ParentNode, turnId: string, occurrence: number): boolean {
  const turn = root.querySelector<HTMLElement>(`[data-turn="${CSS.escape(turnId)}"]`);
  if (!turn) return false;
  const marks = [...turn.querySelectorAll<HTMLElement>(`mark.${MARK}`)];
  const mark = marks[occurrence];
  if (!mark) return false;
  for (const m of root.querySelectorAll<HTMLElement>(`mark.${MARK}[data-active]`)) delete m.dataset.active;
  mark.dataset.active = '';
  mark.scrollIntoView?.({ block: 'center' });
  return true;
}

/** How many highlights are mounted under `root` (the live count, for the bar). */
export function mountedFindCount(root: ParentNode): number {
  return root.querySelectorAll(`mark.${MARK}`).length;
}
