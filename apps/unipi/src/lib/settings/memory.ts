/**
 * Settings remembers where you were for the app session (owner5 #11/#16):
 * the last section, the open extension sub-page and scroll positions. Leaving
 * Settings and coming back lands on the same place; a deep link overrides it.
 * In-memory only (a fresh app start opens Settings at its default).
 */

type Place = { section?: string; namespace?: string; subpage?: string };

let place: Place = {};
const scrolls = new Map<string, number>();

export function rememberPlace(next: Place): void {
  place = { ...next };
}

export function recalledPlace(): Place {
  return { ...place };
}

export function rememberScroll(key: string, top: number): void {
  scrolls.set(key, Math.max(0, Math.round(top)));
}

export function recalledScroll(key: string): number | undefined {
  return scrolls.get(key);
}

/** Tests: forget everything. */
export function resetSettingsMemory(): void {
  place = {};
  scrolls.clear();
}

/** The element that scrolls the Settings page (`.app-content` on the desktop), from any node inside it. */
export function settingsScroller(from: Element | null | undefined): HTMLElement | undefined {
  for (let el = from?.parentElement ?? null; el; el = el.parentElement) {
    if (el.classList.contains('app-content')) return el;
    const style = getComputedStyle(el);
    if (/(auto|scroll)/.test(style.overflowY) && el.scrollHeight > el.clientHeight) return el;
  }
  const root = document.scrollingElement;
  return root instanceof HTMLElement ? root : undefined;
}
