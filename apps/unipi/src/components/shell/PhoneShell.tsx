/**
 * The phone shell (§3.9, WP-A29; Z-115): the unipi frame — bare routes show
 * their own chrome (Chat's header with New chat + "More", the conversation
 * `.ws-layer`, board/cron pages, the Files layer); namespaced routes like
 * Settings get the standard PageHeader. The bottom tab bar (Chat · Board ·
 * Settings) is `TabBar`. While the full-screen editor covers the app the
 * whole frame is `inert`.
 */
import { Show, type ParentProps } from 'solid-js';
import { useLocation, useNavigate } from '@solidjs/router';
import { PageHeader } from '../PageHeader';
import { SettingsNotices } from '../SettingsNotices';
import { UpdateBanner } from '../UpdateCard';
import { TabBar } from './TabBar';

export function PhoneShell(props: ParentProps) {
  const location = useLocation();
  const navigate = useNavigate();
  const onEditor = () => location.pathname === '/files/edit';
  const bare = () => ['/', '/board', '/chat', '/files', '/preview', '/cron'].includes(location.pathname) || onEditor() || location.pathname.startsWith('/s/') || location.pathname.startsWith('/new/');
  const title = () => (location.pathname.startsWith('/settings') ? 'Settings' : 'Chat');
  return (
    <div class="app-shell" inert={onEditor() ? true : undefined} data-testid="phone-shell">
      <Show when={!bare()}>
        <PageHeader title={title()} />
      </Show>
      <main class="app-content">
        <Show when={!location.pathname.startsWith('/settings')}>
          <UpdateBanner onOpen={() => navigate('/settings')} />
        </Show>
        <SettingsNotices />
        {props.children}
      </main>
      <TabBar />
    </div>
  );
}
