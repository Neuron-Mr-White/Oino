/**
 * Settings deep links for non-component code (WP-A25): the built-in routing
 * opens About › What's new (`/changelog`), Providers (`/login`) and Settings
 * itself, then lands on the row the command is about.
 */
import { createSignal } from 'solid-js';
import { appNavigate } from '../nav';

const FLASH_MS = 1400;

/** Top-level Settings section ids (`screens/settings/catalog.ts`). */
const KNOWN_SECTIONS = new Set([
  'general', 'appearance', 'input', 'shortcuts', 'terminal', 'editor', 'board',
  'extensions', 'agent', 'mcp', 'memory', 'skills', 'subagents', 'providers',
  'hosts', 'connection', 'sharing', 'notifications', 'unicrab', 'host', 'about',
]);

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
      el.scrollIntoView?.({ block: anchor.startsWith('sec-') ? 'start' : 'center' });
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
