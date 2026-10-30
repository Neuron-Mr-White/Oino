/**
 * Web build entry (UNI-117 kanboard migration). The kanboard daemon serves
 * this app at `/` and hands out board links in its own shape:
 *   - `/p/<slug>` — `/unipi:kanboard open` and `unipi-kanboard serve` print these,
 *   - `/?project=<slug>&view=list` — the old kanboard web UI's URLs,
 *   - `?t=<token>` — handled by the daemon itself (cookie + 303 redirect
 *     that keeps the path), so it never reaches us.
 * The app routes with a hash (`#/board`), so on load we translate these into
 * "remember this project for the web PC" + `#/board` and tidy the address
 * bar. Hash URLs (`/#/board`) pass through untouched.
 */
import { DEV_HOST_ID } from './board/transport';
import { rememberLast } from './board/ui';

export type WebEntry = { slug?: string; view?: 'board' | 'list'; clean: string } | null;

/** Pure: what the URL asks for (null = nothing to translate). */
export function parseWebEntry(loc: { pathname: string; search: string; hash: string }): WebEntry {
  const params = new URLSearchParams(loc.search);
  const fromPath = /^\/p\/([^/?#]+)\/?$/.exec(loc.pathname)?.[1];
  const slug = fromPath ? decodeURIComponent(fromPath) : (params.get('project') ?? undefined);
  const view = params.get('view') === 'list' ? 'list' : undefined;
  const odd = loc.pathname !== '/' && loc.pathname !== '/index.html';
  if (!slug && !view && !odd) return null;
  params.delete('project');
  params.delete('view');
  params.delete('t');
  const rest = params.toString();
  const hash = loc.hash && loc.hash !== '#' && loc.hash !== '#/' ? loc.hash : '#/board';
  return { slug: slug || undefined, view, clean: `/${rest ? `?${rest}` : ''}${hash}` };
}

/** Applies the entry before the app renders (web mode only). */
export function applyWebEntry(win: Window = window): WebEntry {
  const entry = parseWebEntry(win.location);
  if (!entry) return null;
  if (entry.slug) rememberLast(DEV_HOST_ID, entry.slug);
  if (entry.view) {
    try {
      win.localStorage.setItem('unipi.board.view', entry.view);
    } catch {
      /* storage blocked */
    }
  }
  try {
    win.history.replaceState(null, '', entry.clean);
  } catch {
    /* sandboxed */
  }
  return entry;
}
