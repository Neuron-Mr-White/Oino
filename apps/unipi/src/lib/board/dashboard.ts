/**
 * Dashboard logic (UNI-117 kanboard migration), ported from the kanboard
 * web UI's dashboard.tsx (UNI-67): the cross-project "command center" —
 * needs-you inbox grouped by project, agents running, up next, 14-day
 * throughput + health tiles, clustered activity, project cards. Pure
 * functions; the screen is components/board/Dashboard.tsx.
 */
import type { DashboardData, DashboardInboxItem, ProjectSummary } from './api';

export type InboxFilter = 'all' | 'review' | 'blocked';
export type SortMode = 'attention' | 'recent' | 'name';

/** "3h"/"2d" age of a wait; `old` (≥ 1 day) tints it amber. */
export function waitAge(iso: string, now: number): { label: string; old: boolean } {
  const minutes = Math.max(0, Math.floor((now - Date.parse(iso)) / 60_000));
  const old = minutes >= 24 * 60;
  if (minutes < 60) return { label: `${minutes}m`, old };
  if (minutes < 24 * 60) return { label: `${Math.floor(minutes / 60)}h`, old };
  return { label: `${Math.floor(minutes / (24 * 60))}d`, old };
}

/** "2h ago" for project cards / activity. */
export function relativeAgo(iso: string | null | undefined, now: number): string {
  if (!iso) return '—';
  const minutes = Math.max(0, Math.floor((now - Date.parse(iso)) / 60_000));
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes}m ago`;
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)}h ago`;
  return `${Math.floor(minutes / (24 * 60))}d ago`;
}

export function median(values: number[]): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1 ? sorted[mid]! : Math.round((sorted[mid - 1]! + sorted[mid]!) / 2);
}

/** "30s", "45m", "14h 56m", "2d 3h". */
export function formatDuration(seconds: number): string {
  if (seconds <= 0) return '0s';
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 24 * 3600) {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.round((seconds % 3600) / 60);
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }
  const days = Math.floor(seconds / (24 * 3600));
  const hours = Math.round((seconds % (24 * 3600)) / 3600);
  return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
}

export function greeting(hour: number): string {
  if (hour < 12) return 'Good morning';
  if (hour < 18) return 'Good afternoon';
  return 'Good evening';
}

export function inboxCounts(items: DashboardInboxItem[]): Record<InboxFilter, number> {
  return {
    all: items.length,
    review: items.filter((item) => item.task.status === 'in_review').length,
    blocked: items.filter((item) => item.task.status === 'blocked').length,
  };
}

export function summaryLine(counts: Record<InboxFilter, number>, agents: number): string {
  if (counts.all === 0 && agents === 0) return 'All clear — nothing waiting on you.';
  const parts: string[] = [];
  if (counts.review > 0) parts.push(`${counts.review} review${counts.review === 1 ? '' : 's'}`);
  if (counts.blocked > 0) parts.push(`${counts.blocked} blocked task${counts.blocked === 1 ? '' : 's'}`);
  const waiting = parts.length > 0 ? `${parts.join(' and ')} waiting` : 'nothing waiting';
  return `${waiting} · ${agents} agent${agents === 1 ? '' : 's'} running`;
}

export type InboxGroup = {
  slug: string;
  project: string;
  items: DashboardInboxItem[];
  /** Epoch ms of the oldest wait in the group. */
  oldest: number;
  review: number;
  blocked: number;
  visible: DashboardInboxItem[];
  /** Rows not shown (cap or collapsed). */
  hidden: number;
  collapsed: boolean;
};

/**
 * Inbox grouped by project, groups ordered by their oldest wait; each group
 * shows 3 rows unless expanded, collapsed groups none, and at most 10 rows
 * overall until "Show all".
 */
export function groupInbox(
  items: DashboardInboxItem[],
  filter: InboxFilter,
  prefs: { collapsed: string[]; expanded: string[]; all: boolean },
): InboxGroup[] {
  const filtered = filter === 'all' ? items : items.filter((item) => item.task.status === (filter === 'review' ? 'in_review' : 'blocked'));
  const groups = new Map<string, InboxGroup>();
  for (const item of filtered) {
    const at = Date.parse(item.waitingSince);
    const group = groups.get(item.slug) ?? { slug: item.slug, project: item.project, items: [], oldest: at, review: 0, blocked: 0, visible: [], hidden: 0, collapsed: false };
    group.items.push(item);
    group.oldest = Math.min(group.oldest, at);
    if (item.task.status === 'in_review') group.review += 1;
    if (item.task.status === 'blocked') group.blocked += 1;
    groups.set(item.slug, group);
  }
  let budget = prefs.all ? Number.POSITIVE_INFINITY : 10;
  return [...groups.values()]
    .sort((a, b) => a.oldest - b.oldest)
    .map((group) => {
      const collapsed = prefs.collapsed.includes(group.slug);
      const expanded = prefs.all || prefs.expanded.includes(group.slug);
      const take = collapsed ? 0 : expanded ? group.items.length : Math.min(group.items.length, 3, Math.max(0, budget));
      budget -= take;
      return { ...group, visible: group.items.slice(0, take), hidden: group.items.length - take, collapsed };
    });
}

/** Done per local day for the last 14 days (oldest first). */
export function throughput(doneAt: string[], now: number): Array<{ start: number; label: string; count: number; today: boolean }> {
  const days: Array<{ start: number; label: string; count: number; today: boolean }> = [];
  const stamps = doneAt.map((value) => {
    const day = new Date(value);
    day.setHours(0, 0, 0, 0);
    return day.getTime();
  });
  for (let offset = 13; offset >= 0; offset -= 1) {
    const day = new Date(now);
    day.setHours(0, 0, 0, 0);
    day.setDate(day.getDate() - offset);
    days.push({
      start: day.getTime(),
      label: day.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }),
      count: stamps.filter((stamp) => stamp === day.getTime()).length,
      today: offset === 0,
    });
  }
  return days;
}

export function dashStats(data: DashboardData | null, now: number) {
  const weekAgo = now - 7 * 24 * 3600_000;
  return {
    done7d: (data?.doneAt ?? []).filter((value) => Date.parse(value) >= weekAgo).length,
    medianWait: median(data?.reviewWaits ?? []),
    hasWaits: (data?.reviewWaits ?? []).length > 0,
    blocked: (data?.projects ?? []).reduce((sum, project) => sum + project.blocked, 0),
    repairs: (data?.projects ?? []).reduce((sum, project) => sum + project.problems, 0),
  };
}

/** Client mirror of the backend's status vocabulary: a short verb + the lane it entered. */
export function stepOf(text: string): { label: string; status: string | null } {
  const t = text.trim();
  if (t.startsWith('finished:')) return { label: 'finished', status: 'in_review' };
  if (t.startsWith('released to ')) {
    const status = t.slice(12).split(':')[0]?.trim() ?? '';
    return { label: status === 'in_review' ? 'reviewed' : status, status: status || null };
  }
  if (t === 'blocked' || t.startsWith('blocked:')) return { label: 'blocked', status: 'blocked' };
  if (t.startsWith('unblocked:')) return { label: 'unblocked', status: 'todo' };
  if (t.startsWith('rework:')) return { label: 'rework', status: 'todo' };
  if (t === 'cancelled' || t.startsWith('cancelled:')) return { label: 'cancelled', status: 'cancelled' };
  if (t === 'archived' || t.startsWith('archived')) return { label: 'archived', status: 'archived' };
  if (t.startsWith('moved ')) {
    const status = t.slice(6).split(' → ')[1]?.split(/[: (]/)[0]?.trim() ?? '';
    return { label: status || 'moved', status: status || null };
  }
  if (t.startsWith('undo:')) {
    const status = t.slice(5).split(' → ')[1]?.split(' (')[0]?.trim() ?? '';
    return { label: 'undo', status: status || null };
  }
  if (t.startsWith('created in ')) {
    const status = t.slice(11).split(' ')[0] ?? '';
    return { label: 'created', status: status || null };
  }
  if (t.startsWith('started (')) return { label: 'started', status: 'in_progress' };
  if (t.startsWith('edited ')) return { label: 'edited', status: null };
  return { label: t.split(/[: (]/)[0]?.slice(0, 12) || 'note', status: null };
}

export type ActivityRow = {
  slug: string;
  project: string;
  taskId: string;
  title: string;
  at: string;
  actor: string;
  steps: Array<{ label: string; status: string | null }>;
  lastText: string;
  repeats: number;
};

/** Consecutive entries for the same task within 10 minutes become one row with a step trail. */
export function clusterActivity(activity: DashboardData['activity']): ActivityRow[] {
  const rows: ActivityRow[] = [];
  for (const entry of activity) {
    const step = stepOf(entry.text);
    const last = rows[rows.length - 1];
    if (last && last.slug === entry.slug && last.taskId === entry.taskId && Math.abs(Date.parse(last.at) - Date.parse(entry.at)) <= 10 * 60_000) {
      if (last.steps[0]?.label === 'edited' && step.label === 'edited') {
        last.repeats += 1;
        continue;
      }
      last.steps.unshift(step);
      continue;
    }
    rows.push({ slug: entry.slug, project: entry.project, taskId: entry.taskId, title: entry.title, at: entry.at, actor: entry.actor, lastText: entry.text, repeats: 1, steps: [step] });
  }
  return rows;
}

/** Day buckets ("Today", "Yesterday", "Oct 7") over already-sorted rows. */
export function dayGroups<T extends { at: string }>(rows: T[], now: number): Array<{ key: string; label: string; items: T[] }> {
  const today = new Date(now);
  today.setHours(0, 0, 0, 0);
  const yesterday = today.getTime() - 24 * 3600_000;
  const groups: Array<{ key: string; label: string; items: T[] }> = [];
  for (const row of rows) {
    const day = new Date(row.at);
    day.setHours(0, 0, 0, 0);
    const key = String(day.getTime());
    const label = day.getTime() === today.getTime() ? 'Today' : day.getTime() === yesterday ? 'Yesterday' : day.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    const group = groups.find((candidate) => candidate.key === key);
    if (group) group.items.push(row);
    else groups.push({ key, label, items: [row] });
  }
  return groups;
}

/** First non-empty line of an activity text, attachment images stripped. */
export function activityLine(text: string): string {
  return text.replace(/!\[[^\]]*\]\([^)]*\)/g, '').split('\n').find((line) => line.trim()) ?? '';
}

export function sortProjects(projects: ProjectSummary[], dash: Map<string, DashboardData['projects'][number]>, mode: SortMode): ProjectSummary[] {
  const list = projects.filter((project) => !project.archived);
  if (mode === 'name') return [...list].sort((a, b) => a.name.localeCompare(b.name));
  if (mode === 'recent') return [...list].sort((a, b) => (Date.parse(dash.get(b.slug)?.lastActivity ?? '') || 0) - (Date.parse(dash.get(a.slug)?.lastActivity ?? '') || 0));
  const weight = (project: ProjectSummary) => {
    const entry = dash.get(project.slug);
    return (entry ? entry.blocked * 2 + entry.review : 0) * 1000 + (project.running ?? 0);
  };
  return [...list].sort((a, b) => weight(b) - weight(a) || a.name.localeCompare(b.name));
}
