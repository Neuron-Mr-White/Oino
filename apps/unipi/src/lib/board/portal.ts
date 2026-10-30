/**
 * Chat ↔ kanboard portal (UNI-181).
 *
 *  - Task refs: an exact task id (`UNI-172`) in a chat message becomes a
 *    tappable token. Only prefixes of real projects on that PC count, so
 *    `UTF-8` / `SHA-256` / `GPT-4` stay plain text.
 *  - Board jumps: chat asks the Board screen to open a task (or the New
 *    task sheet) through a one-shot signal; the Board resolves the project
 *    (prefix, tie-broken by the chat's cwd vs the project root), opens it,
 *    then the task — and returns to the chat when that pane closes.
 */
import { createSignal } from 'solid-js';
import { api, type ProjectSummary } from './api';

export type BoardJump = {
  hostId: string;
  /** Open this task's detail pane… */
  taskId?: string;
  /** …or the New task sheet. */
  newTask?: boolean;
  /** The chat session's cwd: picks the project whose root contains it. */
  cwd?: string;
  /** Came from a chat: closing the pane/sheet returns to it. */
  fromChat?: boolean;
};

const [jump, setJump] = createSignal<BoardJump | null>(null);
export const pendingBoardJump = jump;
export function requestBoardJump(next: BoardJump): void {
  setJump(next);
}
export function clearBoardJump(): void {
  setJump(null);
}

/** `UNI-172` → `UNI`; `undefined` when not a task id shape. */
export function refPrefix(ref: string): string | undefined {
  return /^([A-Z][A-Z0-9]{1,9})-\d{1,6}$/.exec(ref)?.[1];
}

function rootContains(root: string | undefined, cwd: string | undefined): number {
  if (!root || !cwd) return -1;
  const r = root.replace(/\/+$/, '');
  return cwd === r || cwd.startsWith(`${r}/`) ? r.length : -1;
}

/** The project a cwd belongs to: the deepest root containing it, else the last opened one. */
export function pickProjectForCwd(projects: ProjectSummary[], cwd?: string, lastSlug?: string): string | undefined {
  const live = projects.filter((p) => !p.archived);
  const byRoot = live
    .map((p) => ({ p, depth: rootContains(p.root, cwd) }))
    .filter((x) => x.depth >= 0)
    .sort((a, b) => b.depth - a.depth)[0];
  if (byRoot) return byRoot.p.slug;
  return live.find((p) => p.slug === lastSlug)?.slug;
}

/** The project a task ref lives in: matching prefix; ties → cwd's root, then the last opened, then the first. */
export function pickProjectForRef(projects: ProjectSummary[], ref: string, cwd?: string, lastSlug?: string): string | undefined {
  const prefix = refPrefix(ref);
  if (!prefix) return undefined;
  const candidates = projects.filter((p) => !p.archived && p.prefix === prefix);
  if (candidates.length <= 1) return candidates[0]?.slug;
  return pickProjectForCwd(candidates, cwd, lastSlug) ?? candidates[0]!.slug;
}

// ── known prefixes per PC (cached; a failed fetch retries next time) ──
const prefixCache = new Map<string, Promise<Set<string>>>();
export function knownPrefixes(hostId: string): Promise<Set<string>> {
  let cached = prefixCache.get(hostId);
  if (!cached) {
    cached = api
      .projects(hostId)
      .then((list) => new Set(list.filter((p) => !p.archived && p.prefix).map((p) => p.prefix!)))
      .catch(() => {
        prefixCache.delete(hostId);
        return new Set<string>();
      });
    prefixCache.set(hostId, cached);
  }
  return cached;
}
/** Test hook. */
export function resetPrefixCache(): void {
  prefixCache.clear();
}

const REF_SCAN = /\b([A-Z][A-Z0-9]{1,9})-(\d{1,6})\b/g;

/** Splits plain text into text runs and task refs with a known prefix. */
export function splitTaskRefs(text: string, prefixes: Set<string>): Array<string | { ref: string }> {
  const out: Array<string | { ref: string }> = [];
  let last = 0;
  for (const match of text.matchAll(REF_SCAN)) {
    if (!prefixes.has(match[1]!)) continue;
    const at = match.index!;
    if (at > last) out.push(text.slice(last, at));
    out.push({ ref: match[0] });
    last = at + match[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/**
 * Wraps task refs in rendered markdown as `<span class="task-ref" data-task>`.
 * Skips code blocks and links; an inline `code` whose whole text is a ref
 * (agents write `UNI-172`) becomes tappable in place. Idempotent.
 */
export function hydrateTaskRefs(root: HTMLElement, prefixes: Set<string>): void {
  if (prefixes.size === 0) return;
  for (const code of root.querySelectorAll<HTMLElement>('code:not([data-task])')) {
    if (code.closest('pre')) continue;
    const text = (code.textContent ?? '').trim();
    const prefix = refPrefix(text);
    if (prefix && prefixes.has(prefix)) {
      code.dataset.task = text;
      code.classList.add('task-ref');
    }
  }
  const walker = root.ownerDocument.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) =>
      (node.parentElement?.closest('pre, code, a, .task-ref, .mmd') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
  });
  const nodes: Text[] = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) nodes.push(node as Text);
  for (const node of nodes) {
    const parts = splitTaskRefs(node.data, prefixes);
    if (parts.length === 1 && typeof parts[0] === 'string') continue;
    const frag = root.ownerDocument.createDocumentFragment();
    for (const part of parts) {
      if (typeof part === 'string') frag.append(part);
      else {
        const span = root.ownerDocument.createElement('span');
        span.className = 'task-ref';
        span.dataset.task = part.ref;
        span.setAttribute('role', 'link');
        span.textContent = part.ref;
        frag.append(span);
      }
    }
    node.replaceWith(frag);
  }
}

// ── board → chat: which chat (UNI-185) ──
/** `/home/u/x` → `~/x`, the form the session list's cwds use. */
const shortHome = (path: string) => path.replace(/^\/home\/[^/]+/, '~').replace(/\/+$/, '');

/**
 * Orders the chats a board can go to: chats whose cwd is under the board
 * project's root first (deepest-first is moot: one root), then the rest,
 * host order kept within each group. `inProject` marks the first group.
 */
export function rankChatsForProject<T extends { cwd?: string }>(chats: readonly T[], root: string | undefined): Array<T & { inProject: boolean }> {
  const r = root ? shortHome(root) : undefined;
  const under = (cwd?: string) => {
    if (!r || !cwd) return false;
    const c = shortHome(cwd);
    return c === r || c.startsWith(`${r}/`);
  };
  const tagged = chats.map((c) => ({ ...c, inProject: under(c.cwd) }));
  return [...tagged.filter((c) => c.inProject), ...tagged.filter((c) => !c.inProject)];
}

/** A request for the Board's chat picker: go to a chat, optionally dropping text into its draft. */
export type ChatPick = { hostId: string; mention?: string };
const [chatPick, setChatPick] = createSignal<ChatPick | null>(null);
export const pendingChatPick = chatPick;
export function requestChatPick(next: ChatPick): void {
  setChatPick(next);
}
export function clearChatPick(): void {
  setChatPick(null);
}
