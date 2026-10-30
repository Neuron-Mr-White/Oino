/**
 * Pure board view logic (UNI-130), ported from the kanboard web UI's
 * state.ts / dep-picker.tsx / mention.tsx / Dialogs.tsx: filtering,
 * lane order + same-lane chain layout, lane visibility, label list,
 * dependency candidates, search scoring. Everything takes its inputs
 * explicitly so it is testable without a store.
 */
import { LANES, displayTitle, type Task } from './api';

export type Filters = { priorities: string[]; labels: string[]; running: boolean; waiting: boolean };
export const EMPTY_FILTERS: Filters = { priorities: [], labels: [], running: false, waiting: false };

export type Density = 'comfortable' | 'compact';
export type DisplayPrefs = {
  /** Hidden lanes (backlog is driven by settings.board.showBacklog instead). */
  hidden: string[];
  excerpt: boolean;
  density: Density;
  /** Draw same-lane dependency chains as adjacent, connected blocks. */
  chains: boolean;
};
export const DEFAULT_DISPLAY: DisplayPrefs = { hidden: ['archived'], excerpt: true, density: 'comfortable', chains: true };

export type View = 'board' | 'list';
/** Shortcuts that narrow the list to one lane (web sidebar: Review queue, Blocked). */
export type Scope = 'all' | 'in_review' | 'blocked';

/** Text query + filters (web `matches`). */
export function matches(task: Task, query: string, filters: Filters): boolean {
  const needle = query.trim().toLowerCase();
  if (
    needle.length > 0 &&
    !(
      task.id.toLowerCase().includes(needle) ||
      task.title.toLowerCase().includes(needle) ||
      (task.displayTitle ?? '').toLowerCase().includes(needle) ||
      (task.body ?? '').toLowerCase().includes(needle) ||
      (task.labels ?? []).some((label) => label.toLowerCase().includes(needle))
    )
  )
    return false;
  if (filters.priorities.length > 0 && !filters.priorities.includes(task.priority ?? 'none')) return false;
  if (filters.labels.length > 0 && !(task.labels ?? []).some((label) => filters.labels.includes(label))) return false;
  if (filters.running && !task.run) return false;
  if (filters.waiting && (task.waitingFor ?? []).length === 0) return false;
  return true;
}

export function filterCount(filters: Filters): number {
  return filters.priorities.length + filters.labels.length + (filters.running ? 1 : 0) + (filters.waiting ? 1 : 0);
}

const byOrder = (a: Task, b: Task) => (a.order ?? 0) - (b.order ?? 0) || a.id.localeCompare(b.id);

/** Matching tasks of one lane in stored order. */
export function laneTasks(tasks: readonly Task[], laneId: string, query: string, filters: Filters): Task[] {
  return tasks.filter((task) => task.status === laneId && matches(task, query, filters)).sort(byOrder);
}

/** Unfiltered lane count (the web's lane header count ignores filters). */
export function laneCount(tasks: readonly Task[], laneId: string): number {
  let count = 0;
  for (const task of tasks) if (task.status === laneId) count += 1;
  return count;
}

export type ChainPos = 'single' | 'first' | 'middle' | 'last';
/** One drawn row of a lane: a task and its place in a same-lane chain. */
export interface LaneItem {
  task: Task;
  chain: ChainPos;
  /** Same-lane parents, drawn above it in the block. */
  parents: string[];
}

/**
 * Lane order with same-lane dependency chains pulled together (web
 * `laneLayout`): a connected component sits where its earliest member is,
 * members are topologically ordered (parents first), ties by stored order.
 */
export function laneLayout(laneTaskList: Task[], chains: boolean): LaneItem[] {
  const tasks = laneTaskList;
  if (!chains || tasks.length < 2) return tasks.map((task) => ({ task, chain: 'single', parents: [] }));
  const here = new Map(tasks.map((task) => [task.id, task]));
  const rank = new Map(tasks.map((task, index) => [task.id, index]));
  const parentsOf = (task: Task): string[] => (task.deps ?? []).filter((dep) => here.has(dep));
  const root = new Map(tasks.map((task) => [task.id, task.id]));
  const find = (id: string): string => {
    let current = id;
    while (root.get(current) !== current) current = root.get(current)!;
    root.set(id, current);
    return current;
  };
  for (const task of tasks) for (const dep of parentsOf(task)) root.set(find(task.id), find(dep));
  const groups = new Map<string, Task[]>();
  for (const task of tasks) {
    const key = find(task.id);
    groups.set(key, [...(groups.get(key) ?? []), task]);
  }
  const out: LaneItem[] = [];
  const placed = new Set<string>();
  for (const task of tasks) {
    const key = find(task.id);
    if (placed.has(key)) continue;
    placed.add(key);
    const members = groups.get(key)!;
    if (members.length === 1) {
      out.push({ task, chain: 'single', parents: [] });
      continue;
    }
    const ids = new Set(members.map((member) => member.id));
    const indegree = new Map(members.map((member) => [member.id, parentsOf(member).filter((dep) => ids.has(dep)).length]));
    const queue = members.filter((member) => indegree.get(member.id) === 0);
    const ordered: Task[] = [];
    while (queue.length > 0) {
      queue.sort((a, b) => rank.get(a.id)! - rank.get(b.id)!);
      const next = queue.shift()!;
      ordered.push(next);
      for (const member of members) {
        if (!parentsOf(member).includes(next.id)) continue;
        const left = indegree.get(member.id)! - 1;
        indegree.set(member.id, left);
        if (left === 0) queue.push(member);
      }
    }
    for (const member of members) if (!ordered.includes(member)) ordered.push(member);
    ordered.forEach((member, index) =>
      out.push({
        task: member,
        chain: index === 0 ? 'first' : index === ordered.length - 1 ? 'last' : 'middle',
        parents: parentsOf(member),
      }),
    );
  }
  return out;
}

/** Lanes shown, in board order. Backlog follows `showBacklog`; scope narrows to one lane. */
export function visibleLanes(display: DisplayPrefs, showBacklog: boolean, scope: Scope = 'all'): { id: string; label: string }[] {
  const lanes = LANES.filter((lane) => (lane.id === 'backlog' ? showBacklog : !display.hidden.includes(lane.id)));
  if (scope !== 'all') {
    const scoped = LANES.filter((lane) => lane.id === scope);
    return scoped.length > 0 ? scoped : lanes;
  }
  return lanes;
}

export function allLabels(tasks: readonly Task[]): string[] {
  const seen = new Set<string>();
  for (const task of tasks) for (const label of task.labels ?? []) seen.add(label);
  return [...seen].sort();
}

/** Newest created first; ties by the id's number, highest first (web mention.tsx). */
export function newestFirst(a: Task, b: Task): number {
  const byDate = Date.parse(b.created ?? '') - Date.parse(a.created ?? '');
  if (byDate) return byDate;
  const num = (id: string): number => Number(/(\d+)$/.exec(id)?.[1] ?? 0);
  return num(b.id) - num(a.id) || b.id.localeCompare(a.id);
}

/** Would adding `depId` to `selfId`'s deps close a cycle? */
export function wouldCycle(tasks: readonly Task[], selfId: string, depId: string): boolean {
  const byId = new Map(tasks.map((task) => [task.id, task]));
  const seen = new Set<string>();
  const stack = [depId];
  while (stack.length > 0) {
    const current = stack.pop()!;
    if (current === selfId) return true;
    if (seen.has(current)) continue;
    seen.add(current);
    for (const next of byId.get(current)?.deps ?? []) stack.push(next);
  }
  return false;
}

export const DEP_MAX_SHOWN = 50;
/** "Runs after" candidates: not cancelled/archived, no cycles, newest first, capped. */
export function depCandidates(
  tasks: readonly Task[],
  needle: string,
  exclude: Set<string>,
  selfId?: string,
): { shown: Task[]; more: number } {
  const text = needle.trim().toLowerCase();
  const pool = tasks
    .filter((task) => !exclude.has(task.id) && task.id !== selfId)
    .filter((task) => !['cancelled', 'archived'].includes(task.status))
    .filter((task) => !selfId || !wouldCycle(tasks, selfId, task.id))
    .filter((task) => !text || task.id.toLowerCase().includes(text) || displayTitle(task).toLowerCase().includes(text))
    .sort(newestFirst);
  return { shown: pool.slice(0, DEP_MAX_SHOWN), more: Math.max(0, pool.length - DEP_MAX_SHOWN) };
}

/** `@` mention candidates (web mention.tsx `recentTasks`). */
export function mentionCandidates(tasks: readonly Task[], needle: string, selfId?: string, max = 8): Task[] {
  const text = needle.trim().toLowerCase();
  return tasks
    .filter((task) => task.id !== selfId && !['archived', 'cancelled'].includes(task.status))
    .filter((task) => !text || task.id.toLowerCase().includes(text) || displayTitle(task).toLowerCase().includes(text))
    .sort((a, b) => mentionRank(a, text) - mentionRank(b, text) || newestFirst(a, b))
    .slice(0, max);
}

/** Exact id, then id prefix, then id substring, then title-only matches (typing "UNI-1" must offer UNI-1 first). */
function mentionRank(task: Task, text: string): number {
  if (!text) return 0;
  const id = task.id.toLowerCase();
  return id === text ? 0 : id.startsWith(text) ? 1 : id.includes(text) ? 2 : 3;
}

/** The `@token` right before the caret, if any (start of text or after whitespace). */
export function mentionAt(text: string, caret: number): { start: number; needle: string } | null {
  const before = text.slice(0, caret);
  const match = /(^|\s)@([\w.-]*)$/.exec(before);
  if (!match) return null;
  return { start: caret - match[2]!.length - 1, needle: match[2]! };
}

export function fuzzy(hay: string, needle: string): boolean {
  let at = 0;
  for (const char of needle) {
    at = hay.indexOf(char, at);
    if (at === -1) return false;
    at += 1;
  }
  return true;
}

/**
 * Task search (the web CommandPalette + toolbar search, merged): id/title
 * substring hits first (by position), then body/label hits, then fuzzy
 * id/title hits only when nothing matched as a substring.
 */
export function searchTasks(tasks: readonly Task[], needle: string, limit = 30): Task[] {
  const text = needle.trim().toLowerCase();
  if (!text) return [...tasks].sort(newestFirst).slice(0, 8);
  const scored: { task: Task; score: number }[] = [];
  for (const task of tasks) {
    const head = `${task.id} ${displayTitle(task)}`.toLowerCase();
    const index = head.indexOf(text);
    if (index !== -1) {
      scored.push({ task, score: index });
      continue;
    }
    const rest = `${task.body ?? ''} ${(task.labels ?? []).join(' ')}`.toLowerCase();
    if (rest.includes(text)) {
      scored.push({ task, score: 1000 });
      continue;
    }
    if (fuzzy(head, text)) scored.push({ task, score: 5000 });
  }
  const exact = scored.filter((entry) => entry.score < 5000);
  return (exact.length > 0 ? exact : scored)
    .sort((a, b) => a.score - b.score || newestFirst(a.task, b.task))
    .slice(0, limit)
    .map((entry) => entry.task);
}

/** "4m", "2h 5m", "3d" — compact elapsed time for the agent chip. */
export function elapsed(iso: string | undefined, now = Date.now()): string {
  if (!iso) return '';
  const start = Date.parse(iso);
  if (Number.isNaN(start)) return '';
  const minutes = Math.max(0, Math.floor((now - start) / 60_000));
  if (minutes < 1) return 'now';
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d`;
}

/** Card excerpt: body with markdown punctuation flattened (web Card). */
export function excerptOf(task: Task): string {
  return (task.body ?? '')
    // Embedded attachments (`![x](att:…)` / `[x](att:…)`) read as their name.
    .replace(/!?\[([^\]]*)\]\(att:[^)]*\)/g, ' $1 ')
    .replace(/[#>*_`[\]]/g, ' ').replace(/\s+/g, ' ').trim();
}

const HUES = [268, 295, 330, 20, 48, 75, 145, 175, 205, 235];
function hash(text: string): number {
  let value = 2166136261;
  for (let index = 0; index < text.length; index += 1) value = Math.imul(value ^ text.charCodeAt(index), 16777619);
  return Math.abs(value);
}
/** A stable hue for a label or project name (same buckets as the web). */
export function hue(text: string): string {
  return `oklch(0.72 0.13 ${HUES[hash(text) % HUES.length]})`;
}
