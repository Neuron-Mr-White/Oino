/**
 * Settings deep links for non-component code (WP-A25): the built-in routing
 * opens About › What's new (`/changelog`), Providers (`/login`) and Settings
 * itself, then lands on the row the command is about.
 */
import { createSignal } from 'solid-js';
import { appNavigate } from '../nav';
import { layout } from '../layout';

const FLASH_MS = 1400;

/** Top-level Settings section ids (`screens/settings/catalog.ts`). */
const KNOWN_SECTIONS = new Set([
  'general', 'appearance', 'input', 'shortcuts', 'terminal', 'editor', 'board',
  'extensions', 'agent', 'mcp', 'memory', 'skills', 'subagents', 'providers',
  'hosts', 'connection', 'sharing', 'notifications', 'unicrab', 'host', 'about',
]);

/**
 * P2-03: extension namespaces that have their own place in the app — opening
 * them goes there, never to a second copy under Settings › Extensions.
 * `[route, anchor?]`.
 */
export const NAMESPACE_HOME: Record<string, { href: string; anchor?: string; label: string }> = {
  notify: { href: '/settings/notifications', anchor: 'sec-notify-pc', label: 'Notifications' },
  cron: { href: '/cron', anchor: 'cron-scheduler', label: 'Cron' },
};

/** Opens a namespace's own place (`NAMESPACE_HOME`); false when it has none. */
export function openNamespaceHome(namespace: string): boolean {
  const home = NAMESPACE_HOME[namespace];
  if (!home) return false;
  setPendingNamespace(undefined);
  if (home.href.startsWith('/settings/')) openSettingsAt(home.href.slice('/settings/'.length), home.anchor);
  else appNavigate(home.href);
  return true;
}

const [pendingNamespace, setPendingNamespace] = createSignal<string | undefined>();

/** Consumes a pending extension settings namespace requested by `openSettings`. */
export function consumePendingSettingsNamespace(): string | undefined {
  const ns = pendingNamespace();
  if (ns !== undefined) setPendingNamespace(undefined);
  return ns;
}

/** Opens `#/settings/<section>` and scrolls to (and flashes) `anchor` once it paints. */
export function openSettingsAt(section: string, anchor?: string): void {
  appNavigate(`/settings/${section}`);
  if (!anchor) return;
  let tries = 0;
  const land = () => {
    const el = document.getElementById(anchor);
    if (el) {
      if (anchor.startsWith('sec-') && layout().desktop()) {
        // F-10: the desktop shows one section per page — land at the very top (title and search visible), not 16 px down.
        for (let p = el.parentElement; p; p = p.parentElement) if (p.scrollTop > 0 || p.classList.contains('app-content')) p.scrollTop = 0;
      } else el.scrollIntoView?.({ block: anchor.startsWith('sec-') ? 'start' : 'center' });
      // owner5 #10: a whole section never flashes (landing on it is feedback enough);
      // a deep-linked row gets the neutral flash.
      if (anchor.startsWith('sec-')) return;
      el.classList.remove('settings-flash-target');
      void el.offsetWidth;
      el.classList.add('settings-flash-target');
      window.setTimeout(() => el.classList.remove('settings-flash-target'), FLASH_MS);
      return;
    }
    tries += 1;
    if (tries < 40) window.setTimeout(land, 60);
  };
  queueMicrotask(land);
}

/**
 * Routes a UI v2 `openSettings` effect (`{namespace?, section?, query?}`) to
 * its Settings section: dedicated capability pages (`skills`, `mcp`, `memory`,
 * `subagents`) and known sections open directly; `oino.*` opens Agent; other
 * extension namespaces open Settings › Extensions on that namespace's form.
 */
export function openSettingsFor(opts: { namespace?: string; section?: string; query?: string } = {}): void {
  const rawSection = opts.section?.trim();
  const rawNs = opts.namespace?.trim();
  if (rawSection && KNOWN_SECTIONS.has(rawSection)) {
    setPendingNamespace(undefined);
    openSettingsAt(rawSection, `sec-${rawSection}`);
    return;
  }
  if (rawNs) {
    if (KNOWN_SECTIONS.has(rawNs)) {
      setPendingNamespace(undefined);
      openSettingsAt(rawNs, `sec-${rawNs}`);
      return;
    }
    if (openNamespaceHome(rawNs)) return;
    if (rawNs.startsWith('oino.')) {
      setPendingNamespace(undefined);
      openSettingsAt('agent', 'sec-agent');
      return;
    }
    setPendingNamespace(rawNs);
    openSettingsAt('extensions', `ns-${rawNs}`);
    return;
  }
  setPendingNamespace(undefined);
  appNavigate('/settings');
}
