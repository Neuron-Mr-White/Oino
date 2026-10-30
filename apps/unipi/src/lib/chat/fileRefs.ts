/**
 * Tappable file references in chat (UNI-204 rework).
 *
 * 1. `fileRefScan.ts` finds path-shaped candidates in text (prose, inline
 *    and fenced code, tool output, markdown link targets);
 * 2. `pathStat.ts` asks the bridge which exist on the PC (`paths_stat`,
 *    one batched request, cached per chat) and where they resolve to;
 * 3. only the existing ones become `.file-ref` links here — a path that
 *    isn't on the PC stays plain text.
 *
 * `hydrateFileRefs` does it for rendered markdown (MdBlock); `fileRefHits`
 * + `splitByHits` do it for plain text the Solid tree renders (user
 * bubbles, tool output). Taps/long-press go through `fileRefOpen.ts`.
 */
import { joinBase, scanFileRefs, type FileRefCandidate, type ScanMode } from './fileRefScan';
import { lookupPaths, pathStatUnavailable, type StatChat } from './pathStat';
import { isExternalHref } from '../markdown/links';

const BLOB_DIR_RE = /\/\.unipi\/app-host\/blobs\/([0-9a-f]{16,64})(?:[/?#]|$)/i;

/** A verified file reference: a span of the scanned text and where it lives on the PC. */
export type FileRefHit = { start: number; end: number; path: string; resolved: string; kind: 'file' | 'dir'; line?: number; col?: number };

/** The paths to ask the bridge about for one candidate: joined onto `base` first (tool output relative
 * to the tool's own `path` argument), then as written. */
function asks(c: FileRefCandidate, base?: string): string[] {
  const joined = joinBase(c.path, base);
  return joined === c.path ? [c.path] : [joined, c.path];
}

/**
 * The pre-UNI-204 fallback for a chat whose pi can't answer `paths_stat`
 * (older bridge, timeout): an absolute path (`/abs/…`, `file:///…`) is
 * linked unverified, as the app did before; anything relative stays plain
 * (it can't be resolved reliably without the PC).
 */
export function unverifiedHit(c: FileRefCandidate): FileRefHit | undefined {
  if (!c.path.startsWith('/') || c.path.length < 2) return undefined;
  const dir = c.path.endsWith('/');
  const resolved = dir ? c.path.replace(/(.)\/+$/, '$1') : c.path;
  return { start: c.start, end: c.end, path: c.path, resolved, kind: dir ? 'dir' : 'file', line: c.line, col: c.col };
}

/** Verifies candidates against the PC: one answer per candidate (same order), `undefined` when it
 * isn't an existing file/dir. When the bridge can't verify at all (`pathStatUnavailable`), absolute
 * paths fall back to `unverifiedHit`. */
export async function verifyEach(chat: StatChat, cands: readonly FileRefCandidate[], base?: string): Promise<Array<FileRefHit | undefined>> {
  if (cands.length === 0) return [];
  const stats = await lookupPaths(chat, cands.flatMap((c) => asks(c, base)));
  const fallback = pathStatUnavailable(chat);
  return cands.map((c) => {
    const hit = asks(c, base)
      .map((p) => stats.get(p))
      .find((s) => s && s.kind !== 'missing');
    if (!hit && fallback) return unverifiedHit(c);
    if (!hit || hit.kind === 'missing') return undefined;
    return { start: c.start, end: c.end, path: c.path, resolved: hit.resolved, kind: hit.kind, line: c.line, col: c.col };
  });
}

/** The existing ones only. */
export async function verifyCandidates(chat: StatChat, cands: readonly FileRefCandidate[], base?: string): Promise<FileRefHit[]> {
  return (await verifyEach(chat, cands, base)).filter((h): h is FileRefHit => !!h);
}

/** Scan + verify plain text (bubbles, tool output). */
export function fileRefHits(chat: StatChat, text: string, mode: ScanMode, base?: string): Promise<FileRefHit[]> {
  return verifyCandidates(chat, scanFileRefs(text, mode), base);
}

/** Splits `text` into plain runs and verified refs (hits must be sorted, non-overlapping). */
export function splitByHits(text: string, hits: readonly FileRefHit[]): Array<string | { hit: FileRefHit; text: string }> {
  const out: Array<string | { hit: FileRefHit; text: string }> = [];
  let at = 0;
  for (const h of hits) {
    if (h.start < at) continue;
    if (h.start > at) out.push(text.slice(at, h.start));
    out.push({ hit: h, text: text.slice(h.start, h.end) });
    at = h.end;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}

/** `~/.unipi/app-host/blobs/<sha256>` → `attachment·<first 8 of sha>`;
 * otherwise the basename, with the parent dir available separately for
 * dimmed display (attachment/path chips, UNI-203/212). */
export function fileRefLabel(path: string): { label: string; dim?: string } {
  const blob = BLOB_DIR_RE.exec(path);
  if (blob) return { label: `attachment·${blob[1]!.slice(0, 8)}` };
  const clean = path.replace(/\/+$/, '');
  const slash = clean.lastIndexOf('/');
  if (slash < 0) return { label: clean };
  const base = clean.slice(slash + 1);
  const dir = clean.slice(0, slash + 1);
  if (!base) return { label: clean };
  return { label: base, dim: dir };
}

/** Data attributes every file ref carries (span, inline code or anchor). */
export function setRefData(el: HTMLElement, hit: FileRefHit): void {
  el.dataset.fileRef = hit.resolved;
  el.dataset.fileKind = hit.kind;
  if (hit.line) el.dataset.fileLine = String(hit.line);
  else delete el.dataset.fileLine;
  el.title = hit.line ? `${hit.resolved}:${hit.line}` : hit.resolved;
}

/** Reads a ref back off an element (click/long-press handlers). */
export function refFromElement(el: HTMLElement): { resolved: string; kind: 'file' | 'dir'; line?: number } | undefined {
  const resolved = el.dataset.fileRef;
  if (!resolved) return undefined;
  const line = Number(el.dataset.fileLine);
  return { resolved, kind: el.dataset.fileKind === 'dir' ? 'dir' : 'file', line: Number.isFinite(line) && line > 0 ? line : undefined };
}

type Unit =
  | { kind: 'anchor'; el: HTMLAnchorElement; cands: FileRefCandidate[] }
  | { kind: 'code'; el: HTMLElement; nodes: Text[]; offsets: number[]; text: string; cands: FileRefCandidate[] }
  | { kind: 'text'; node: Text; cands: FileRefCandidate[] };

/** Prose text nodes to scan: not inside code (scanned separately), links (their target is), task refs,
 * mermaid sources, code-block chrome, buttons, or an existing ref. */
const PROSE_SKIP = 'pre, code, a, .task-ref, .mmd, .file-ref, .cb-head, button, .att-chip, .att-image';

function textNodes(root: Node, accept?: (n: Text) => boolean): Text[] {
  const doc = root.ownerDocument ?? (root as Document);
  const walker = doc.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) => (!accept || accept(n as Text) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT),
  });
  const out: Text[] = [];
  for (let n = walker.nextNode(); n; n = walker.nextNode()) out.push(n as Text);
  return out;
}

/** A markdown link target that names a local path: `[x](src/a.ts)`, `[x](./a.md#L3)`, `<file:///…>`. */
function anchorCandidate(a: HTMLAnchorElement): FileRefCandidate | undefined {
  if (a.dataset.attPath || a.closest('.att-chip, .att-image')) return undefined;
  let href = a.getAttribute('href') ?? '';
  if (!href || href.startsWith('#') || (isExternalHref(href) && !/^file:/i.test(href))) return undefined;
  try {
    href = decodeURI(href);
  } catch {
    // keep as is
  }
  const c = scanFileRefs(href, 'code');
  if (c.length !== 1 || c[0]!.start !== 0 || c[0]!.end < href.replace(/#.*$/, '').length) return undefined;
  return c[0];
}

function collect(root: HTMLElement): Unit[] {
  const units: Unit[] = [];
  for (const a of root.querySelectorAll<HTMLAnchorElement>('a[href]:not([data-file-ref])')) {
    const c = anchorCandidate(a);
    if (c) units.push({ kind: 'anchor', el: a, cands: [c] });
  }
  for (const code of root.querySelectorAll<HTMLElement>('code')) {
    if (code.classList.contains('task-ref') || code.dataset.fileRef || code.closest('.mmd, a')) continue;
    if (code.querySelector('.file-ref')) continue; // already hydrated
    const nodes = textNodes(code);
    const offsets: number[] = [];
    let text = '';
    for (const n of nodes) {
      offsets.push(text.length);
      text += n.data;
    }
    const cands = scanFileRefs(text, 'code');
    if (cands.length) units.push({ kind: 'code', el: code, nodes, offsets, text, cands });
  }
  for (const node of textNodes(root, (n) => !n.parentElement?.closest(PROSE_SKIP))) {
    const cands = scanFileRefs(node.data, 'prose');
    if (cands.length) units.push({ kind: 'text', node, cands });
  }
  return units;
}

function makeSpan(doc: Document, hit: FileRefHit): HTMLSpanElement {
  const span = doc.createElement('span');
  span.className = 'file-ref';
  span.setAttribute('role', 'link');
  setRefData(span, hit);
  return span;
}

/** Wraps [start,end) of the concatenated text of `nodes` (at `offsets`) in ref spans — one per text node
 * the range touches (highlighting may split a path across token spans). */
function wrapRange(nodes: Text[], offsets: number[], hit: FileRefHit): void {
  for (let i = nodes.length - 1; i >= 0; i--) {
    const node = nodes[i]!;
    const from = offsets[i]!;
    const to = from + node.data.length;
    const s = Math.max(hit.start, from);
    const e = Math.min(hit.end, to);
    if (s >= e) continue;
    const mid = s - from > 0 ? node.splitText(s - from) : node;
    if (e - s < mid.data.length) mid.splitText(e - s);
    const span = makeSpan(node.ownerDocument, hit);
    mid.replaceWith(span);
    span.append(mid);
  }
}

/**
 * Finds, verifies and wraps file refs in rendered markdown (MdBlock).
 * Call after code highlighting (which replaces code contents) and task-ref
 * hydration. `isCurrent` lets the caller drop a stale pass (the block
 * re-rendered while the stat was in flight). Idempotent.
 */
export async function hydrateFileRefs(root: HTMLElement, chat: StatChat, isCurrent: () => boolean = () => true): Promise<void> {
  const units = collect(root);
  if (!units.length) return;
  const all = units.flatMap((u) => u.cands);
  const answers = await verifyEach(chat, all);
  if (!isCurrent() || !answers.some(Boolean)) return;
  const byCand = new Map<FileRefCandidate, FileRefHit>();
  all.forEach((c, i) => answers[i] && byCand.set(c, answers[i]!));
  for (const u of units) {
    const found = u.cands.map((c) => byCand.get(c)).filter((h): h is FileRefHit => !!h);
    if (!found.length) continue;
    if (u.kind === 'anchor') {
      setRefData(u.el, found[0]!);
      u.el.classList.add('file-ref-link');
      continue;
    }
    if (u.kind === 'code') {
      const only = found.length === 1 ? found[0]! : undefined;
      const inline = !u.el.closest('pre');
      // Inline code that IS the path: the whole `<code>` is the link.
      if (inline && only && u.text.slice(0, only.start).trim() === '' && u.text.slice(only.end).trim() === '') {
        setRefData(u.el, only);
        u.el.classList.add('file-ref-code');
        continue;
      }
      for (const h of [...found].sort((a, b) => b.start - a.start)) wrapRange(u.nodes, u.offsets, h);
      continue;
    }
    for (const h of [...found].sort((a, b) => b.start - a.start)) wrapRange([u.node], [0], h);
  }
}
