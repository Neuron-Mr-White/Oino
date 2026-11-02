/**
 * The phone shell (phone spec §1): the frame every phone route renders in —
 * the page column and the five-tab bar (`TabBar`). Pages bring their own
 * chrome (`PageScaffold`, components/phone); routes that do not yet get the
 * legacy PageHeader. The tab bar hides on workspace pages (chat, new chat,
 * editor, task detail), while a state-driven layer asks it to
 * (`setTabBarHidden`), and while the keyboard is open.
 *
 * Back is hierarchical (§1.3): with no overlay open, Android Back goes to the
 * route's parent (`phoneParent`), tab roots go to Chats, and Chats sends the
 * app to the background. No update banner on tab pages (PH-13): one toast per
 * version instead (`PhoneUpdateToast`).
 */
import { createEffect, onCleanup, Show, type ParentProps } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { PageHeader } from '../PageHeader';
import { SettingsNotices } from '../SettingsNotices';
import { PhoneUpdateToast } from '../UpdateCard';
import { TabBar } from './TabBar';
import { setRootBack } from '../../lib/back';
import { isTauriMobile } from '../../lib/platform';
import { isWorkspace, phoneParent } from '../../lib/phoneNav';
import { useSettings } from '../../settings/context';
import { setHapticsEnabled, tabBarHiddenByLayer, useKeyboard } from '../phone/services';

/** Sends the app to the background (Android `moveTaskToBack`, §1.3 rule 5). */
export function backgroundApp(): void {
  const bridge = (window as Window & { UniPiApp?: { background: () => void } }).UniPiApp;
  try {
    bridge?.background();
  } catch {
    // no bridge (browser / old APK): stay on Chats
  }
}

/** Pure: UI font size (12–20, default 15) → the phone type scale (0.85–1.25). */
export function phoneScale(uiFontSize: number | undefined): number {
  const px = typeof uiFontSize === 'number' && Number.isFinite(uiFontSize) ? uiFontSize : 15;
  return Math.round(Math.min(1.25, Math.max(0.85, px / 15)) * 100) / 100;
}

/** Mirrors phone-relevant settings into the phone services (haptics, UI scale). */
function PhoneServices() {
  const [settings] = useSettings();
  createEffect(() => setHapticsEnabled(settings.input.haptics));
  // §2.2: the "UI font size" setting (default 15) scales the phone type tokens.
  createEffect(() => document.documentElement.style.setProperty('--ph-scale', String(phoneScale(settings.appearance.uiFontSize))));
  onCleanup(() => document.documentElement.style.removeProperty('--ph-scale'));
  return null;
}

export function PhoneShell(props: ParentProps) {
  const location = useLocation();
  const navigate = useNavigate();
  const keyboard = useKeyboard();
  const onEditor = () => location.pathname === '/files/edit';
  // Routes whose screen draws its own header (all tab pages and workspaces); the rest get PageHeader.
  const bare = () =>
    ['/', '/board', '/chat', '/files', '/preview', '/cron'].includes(location.pathname) ||
    location.pathname.startsWith('/chat/') ||
    location.pathname.startsWith('/cron/') ||
    location.pathname.startsWith('/board/') ||
    location.pathname.startsWith('/x/') ||
    onEditor() ||
    location.pathname.startsWith('/s/') ||
    location.pathname.startsWith('/new/');
  const title = () => (location.pathname.startsWith('/settings') ? 'Settings' : 'Chats');
  const tabBarHidden = () => isWorkspace(location.pathname) || tabBarHiddenByLayer() || keyboard.open();

  createEffect(() => {
    const root = document.documentElement;
    if (tabBarHidden()) root.dataset.tabbar = 'hidden';
    else delete root.dataset.tabbar;
  });
  onCleanup(() => delete document.documentElement.dataset.tabbar);

  // §1.3: route-level Back (only the phone app has a hardware Back).
  if (isTauriMobile()) {
    setRootBack(() => {
      const parent = phoneParent(location.pathname, location.search);
      if (parent === undefined) backgroundApp();
      else navigate(parent, { replace: true });
    });
    onCleanup(() => setRootBack(undefined));
  }

  return (
    <div class="app-shell ph-shell" inert={onEditor() ? true : undefined} data-testid="phone-shell">
      <PhoneServices />
      <Show when={!bare()}>
        <PageHeader title={title()} class="ph-legacy-head" />
      </Show>
      <main class="app-content">
        <SettingsNotices />
        {props.children}
      </main>
      <Show when={!tabBarHidden()}>
        <TabBar />
      </Show>
      <PhoneUpdateToast onOpen={() => navigate('/settings')} />
    </div>
  );
}
