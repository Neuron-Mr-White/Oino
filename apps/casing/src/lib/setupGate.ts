/**
 * UNI-241: when the desktop app shows the full-screen `#/setup` takeover.
 *
 * §4.3: first run only, desktop only, and only when detection says this PC
 * isn't ready. "Not now" is remembered (localStorage) with a long expiry, so
 * pressing it once doesn't haunt every launch — and while it is remembered,
 * Settings → This computer carries a banner in the takeover's place
 * ([`setupBannerVisible`]).
 *
 * The decision itself is `setupGateOpen` in `lib/setup.ts`, a pure function
 * over the report; this file is the navigator effect that acts on it.
 */
import { createSignal, onCleanup, onMount } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { dismissedAt, refreshSetup, setupGateOpen, setupReport } from './setup';

/** Once "Not now" has been pressed, the takeover is replaced by a banner. */
export function setupBannerVisible(report = setupReport(), dismissed = dismissedAt()): boolean {
  return !!report && !report.ready && !setupGateOpen(report, dismissed);
}

/**
 * Mounted once in the shell: detect this PC, and if it isn't ready and the
 * takeover hasn't been dismissed, send the desktop app to `#/setup`.
 *
 * It only ever navigates *from* the app's own landing route, so a deep link,
 * a restored window position or the phone's routes are never hijacked. The
 * gate is also only checked once per launch: "Not now" and the banner are how
 * a user opts out, not a lock.
 */
export function SetupGate() {
  const navigate = useNavigate();
  const location = useLocation();
  const [opened, setOpened] = createSignal(false);
  onMount(async () => {
    const report = await refreshSetup();
    if (!report || opened()) return;
    if (location.pathname !== '/') return;
    if (!setupGateOpen(report, dismissedAt(), Date.now(), true)) return;
    setOpened(true);
    navigate('/setup', { replace: true });
  });
  onCleanup(() => setOpened(false));
  return null;
}
