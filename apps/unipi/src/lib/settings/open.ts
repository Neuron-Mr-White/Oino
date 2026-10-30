/**
 * Settings deep links for non-component code (WP-A25): the built-in routing
 * opens About › What's new (`/changelog`), Providers (`/login`) and Settings
 * itself, then lands on the row the command is about.
 */
import { appNavigate } from '../nav';

const FLASH_MS = 1400;

/** Opens `#/settings/<section>` and scrolls to (and flashes) `anchor` once it paints. */
export function openSettingsAt(section: string, anchor?: string): void {
  appNavigate(`/settings/${section}`);
  if (!anchor) return;
  let tries = 0;
  const land = () => {
    const el = document.getElementById(anchor);
    if (el) {
      el.scrollIntoView?.({ block: 'center' });
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
