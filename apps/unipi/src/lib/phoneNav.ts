/**
 * Phone navigation model (phone spec §1): the five tabs, which routes are
 * workspaces (tab bar hidden), and the hierarchical Back parent of every
 * route — computed from the route, never from history (§1.3, PH-17).
 * Package A owns this table; B and C ask for additions.
 */

import { taskBackTarget } from './board/taskQueue';

/** `settings` is not in the bar (its gear sits in the Chats header) but still owns its routes. */
export type PhoneTab = 'chat' | 'board' | 'superagent' | 'cron' | 'files' | 'settings';

/** The bottom bar, left to right: Superagent is the centred circle. */
export const PHONE_TABS: readonly PhoneTab[] = ['chat', 'board', 'superagent', 'cron', 'files'];

/** Tab roots. */
export const TAB_ROOT: Record<PhoneTab, string> = {
  chat: '/chat',
  board: '/board',
  superagent: '/superagent',
  cron: '/cron',
  files: '/files',
  settings: '/settings',
};

/** Pure: which tab a route belongs to (undefined: none, e.g. `/`). */
export function tabOf(pathname: string): PhoneTab | undefined {
  if (pathname === '/superagent') return 'superagent';
  if (pathname === '/chat' || pathname.startsWith('/chat/') || pathname.startsWith('/s/') || pathname.startsWith('/new/')) return 'chat';
  if (pathname === '/board' || pathname.startsWith('/board/')) return 'board';
  if (pathname === '/cron' || pathname.startsWith('/cron/')) return 'cron';
  if (pathname === '/files' || pathname.startsWith('/files/')) return 'files';
  if (pathname === '/settings' || pathname.startsWith('/settings/') || pathname.startsWith('/x/') || pathname === '/preview') return 'settings';
  return undefined;
}

/** Pure: is this route a tab root? */
export function isTabRoot(pathname: string, search = ''): boolean {
  if (pathname === '/files') return !filesPath(search) || filesPath(search) === filesRoot(search);
  return (Object.values(TAB_ROOT) as string[]).includes(pathname);
}

/** Pure: a workspace page hides the tab bar (chat, new chat, editor, task detail…, §1.2). */
export function isWorkspace(pathname: string): boolean {
  return (
    pathname === '/superagent' ||
    pathname.startsWith('/s/') ||
    pathname.startsWith('/new/') ||
    pathname === '/files/edit' ||
    pathname.startsWith('/board/task/')
  );
}

function params(search: string): URLSearchParams {
  return new URLSearchParams(search.startsWith('?') ? search.slice(1) : search);
}
function filesPath(search: string): string | undefined {
  return params(search).get('path') ?? undefined;
}
function filesRoot(search: string): string | undefined {
  return params(search).get('root') ?? undefined;
}

/** Pure: `/a/b/c` → `/a/b`; `~/x` → `~`; roots (`/`, `~`) → undefined. */
export function parentFolder(path: string): string | undefined {
  const trimmed = path.length > 1 ? path.replace(/\/+$/, '') : path;
  if (trimmed === '/' || trimmed === '~' || trimmed === '') return undefined;
  const at = trimmed.lastIndexOf('/');
  if (at < 0) return undefined;
  if (at === 0) return '/';
  return trimmed.slice(0, at);
}

/**
 * Pure: the route Back goes to from `pathname` + `search` (§1.2 table).
 * `undefined` = this is `/chat` (or unknown and already home): background the app.
 */
export function phoneParent(pathname: string, search = ''): string | undefined {
  if (pathname === '/chat' || pathname === '/') return undefined;
  if (pathname.startsWith('/chat/')) return '/chat';
  if (pathname.startsWith('/s/') || pathname.startsWith('/new/')) return '/chat';
  if (pathname.startsWith('/board/')) return taskBackTarget(params(search).get('from') ?? undefined);
  if (pathname.startsWith('/cron/')) return '/cron';
  if (pathname === '/files/edit') {
    const from = params(search).get('from') ?? (filesPath(search) ? parentFolder(filesPath(search)!) : undefined);
    return from ? `/files?path=${encodeURIComponent(from)}` : '/files';
  }
  if (pathname === '/files') {
    const path = filesPath(search);
    if (!path) return '/chat';
    const up = parentFolder(path);
    const root = filesRoot(search);
    // Above the folder the user started from (a Place), or at a filesystem root: the Places list.
    if (!up || (root && path === root)) return '/files';
    return `/files?path=${encodeURIComponent(up)}${root ? `&root=${encodeURIComponent(root)}` : ''}`;
  }
  if (pathname.startsWith('/settings/')) {
    const parts = pathname.split('/').filter(Boolean); // settings, section, sub?
    return parts.length > 2 ? `/settings/${parts[1]}` : '/settings';
  }
  if (pathname.startsWith('/x/') || pathname === '/preview') return '/settings';
  // Tab roots other than Chats, and anything unknown, go home.
  return '/chat';
}
