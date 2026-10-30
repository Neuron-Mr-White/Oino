/**
 * Find highlighting over the virtual timeline (WP-A18, Z-79): DOM-level, so
 * the static-once-complete markdown (Z-34) and the virtualizer stay as they
 * are. `applyFindHighlights` marks each case-insensitive hit in
 * `<mark class="find-hit">`; the active match gets `data-active` and scrolls
 * into view. `clearFindHighlights` puts the plain text back.
 *
 * The bubble/prose text is rendered by Solid, which re-writes the text nodes
 * it owns (a late `knownPrefixes` resource, a virtualizer re-render). A split
 * text node would then get its full text back *next to* the marks, so a pass
 * over the mutated DOM would double the text. Each element we touch therefore
 * remembers its plain inner HTML; every pass rebuilds from that copy rather
 * than from the (possibly re-rendered) live nodes.
 */

const MARK = 'find-hit';

/** Elements rewritten in place → their plain (unmarked) HTML. */
const plain = new WeakMap<Element, string>();
/** The elements above (a WeakMap is not iterable) so a clear can restore them. */
const touched = new Set<Element>();

/** The element's inner HTML when every child is a text node (no handlers to lose). */
function textOnly(el: Element): string | undefined {
  for (const child of el.childNodes) if (child.nodeType !== Node.TEXT_NODE) return undefined;
  return el.innerHTML;
}

/** Restores one element's plain content (and forgets it). */
function restore(el: Element): void {
  const html = plain.get(el);
  touched.delete(el);
  if (html === undefined) return;
  plain.delete(el);
  el.innerHTML = html;
}

/** Removes every highlight under `root`, restoring plain text nodes. */
export function clearFindHighlights(root: ParentNode): void {
  for (const el of [...touched]) {
    if (root.contains(el)) restore(el);
  }
  // Mixed-content parents (and anything a re-render put back): unwrap the marks.
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
  const byParent = new Map<HTMLElement, Text[]>();
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const el = (n as Text).parentElement!;
    const list = byParent.get(el);
    if (list) list.push(n as Text);
    else byParent.set(el, [n as Text]);
  }
  let count = 0;
  for (const [el, nodes] of byParent) {
    const saved = textOnly(el);
    if (saved === undefined) {
      // Mixed children (inline markup): wrap the text node in place.
      for (const node of nodes) count += wrapMatches(node, needle);
      continue;
    }
    // Rebuild the element from its plain text so a Solid re-render that reset
    // a text node we split can neither duplicate nor lose the text. When the
    // marks are gone the framework re-rendered the element: take the new text.
    let html = plain.get(el) ?? saved;
    if (plain.has(el)) {
      if (el.querySelector(`mark.${MARK}`)) el.innerHTML = html;
      else {
        html = saved;
        plain.set(el, saved);
      }
    } else {
      plain.set(el, saved);
    }
    touched.add(el);
    const frag = document.createDocumentFragment();
    for (const child of [...el.childNodes]) {
      if (child.nodeType !== Node.TEXT_NODE || !child.nodeValue?.toLowerCase().includes(needle)) {
        frag.append(child.cloneNode());
        continue;
      }
      count += appendMatched(frag, child.nodeValue, needle);
    }
    el.replaceChildren(frag);
  }
  return count;
}

/** Appends `text` to `frag` with each hit wrapped in a mark; returns the hits. */
function appendMatched(frag: DocumentFragment, text: string, needle: string): number {
  let count = 0;
  let at = text.toLowerCase().indexOf(needle);
  let from = 0;
  while (at >= 0) {
    if (at > from) frag.append(text.slice(from, at));
    const mark = document.createElement('mark');
    mark.className = MARK;
    mark.textContent = text.slice(at, at + needle.length);
    frag.append(mark);
    count += 1;
    from = at + needle.length;
    at = text.toLowerCase().indexOf(needle, from);
  }
  if (from < text.length) frag.append(text.slice(from));
  return count;
}

/** Wraps the hits of one text node in place (mixed-content parents). */
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
export function activateFindMark(root: ParentNode, turnId: string, occurrence: number, scroll = true): boolean {
  const turn = root.querySelector<HTMLElement>(`[data-turn="${CSS.escape(turnId)}"]`);
  if (!turn) return false;
  const marks = [...turn.querySelectorAll<HTMLElement>(`mark.${MARK}`)];
  const mark = marks[occurrence];
  if (!mark) return false;
  for (const m of root.querySelectorAll<HTMLElement>(`mark.${MARK}[data-active]`)) delete m.dataset.active;
  mark.dataset.active = '';
  if (scroll) mark.scrollIntoView?.({ block: 'center' });
  return true;
}

/** How many highlights are mounted under `root` (the live count, for the bar). */
export function mountedFindCount(root: ParentNode): number {
  return root.querySelectorAll(`mark.${MARK}`).length;
}
