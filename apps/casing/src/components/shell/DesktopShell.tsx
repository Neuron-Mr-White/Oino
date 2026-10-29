/**
 * Desktop shell pieces (UNI-117): the left sidebar (replaces the bottom tab
 * bar at ≥ 900px outside the phone app), the list | detail splitter and
 * placeholder, the global keyboard shortcuts and the html data attributes
 * the desktop CSS keys off. See lib/layout.ts + lib/shell.ts.
 */
import { createEffect, createMemo, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { A, useLocation, useNavigate } from '@solidjs/router';
import { Terminal, Columns3, Settings as SettingsIcon, MessagesSquare, Files as FilesIcon, Globe, Command as CommandIcon, Search, Clock } from 'lucide-solid';
import { can, isWeb, type Capability } from '../../lib/platform';
import { layout, SIDEBAR_WIDTH } from '../../lib/layout';
import { modKey, paneFor, paletteOpen, registerCommands, screenOf, setPaletteOpen } from '../../lib/shell';
import { syncChatVisibleOnScreenChange } from '../../lib/chat/open';
import crab from '../../assets/unicrab-pixel.png';
import { EmptyState } from '../EmptyState';
import { setShortcutsOpen, shortcutsOpen } from './ShortcutsDialog';
import { Keyboard } from 'lucide-solid';

export type NavItem = { href: string; label: string; screen: string; cap?: Capability; icon: (size: number) => JSX.Element };

/** The main screens, in sidebar / Ctrl+1…9 order. */
export const NAV: NavItem[] = [
  { href: '/', label: 'Sessions', screen: 'sessions', cap: 'terminal', icon: (s) => <Terminal size={s} strokeWidth={1.6} /> },
  { href: '/chat', label: 'Chat', screen: 'chat', cap: 'chat', icon: (s) => <MessagesSquare size={s} strokeWidth={1.6} /> },
  { href: '/board', label: 'Board', screen: 'board', cap: 'board', icon: (s) => <Columns3 size={s} strokeWidth={1.6} /> },
  { href: '/files', label: 'Files', screen: 'files', cap: 'files', icon: (s) => <FilesIcon size={s} strokeWidth={1.6} /> },
  { href: '/cron', label: 'Cron', screen: 'cron', icon: (s) => <Clock size={s} strokeWidth={1.6} /> },
  { href: '/preview', label: 'Preview', screen: 'preview', cap: 'portForward', icon: (s) => <Globe size={s} strokeWidth={1.6} /> },
  { href: '/settings', label: 'Settings', screen: 'settings', icon: (s) => <SettingsIcon size={s} strokeWidth={1.6} /> },
];

/** Usable in this platform mode? (Unavailable screens still open, showing the notice.) */
export function navAvailable(item: NavItem): boolean {
  return !item.cap || can(item.cap);
}

export function Sidebar(props: { footer?: JSX.Element }) {
  const location = useLocation();
  const active = (item: NavItem) => screenOf(location.pathname) === item.screen;
  return (
    <aside class="shell-sidebar" aria-label="Main navigation">
      <div class="shell-brand">
        <img src={crab} alt="" class="shell-brand-crab" />
        <span class="shell-brand-word">UniPi</span>
        <Show when={isWeb()}>
          <span class="shell-brand-badge" title="Running in a browser: only the board is available">web</span>
        </Show>
      </div>
      <button type="button" class="shell-search" onClick={() => setPaletteOpen(true)} aria-label="Command palette">
        <Search size={15} />
        <span>Search</span>
        <span class="shell-search-keys">
          <kbd class="kbd">{modKey()}</kbd>
          <kbd class="kbd">K</kbd>
        </span>
      </button>
      <nav class="shell-nav">
        <For each={NAV}>
          {(item, index) => (
            <A
              href={item.href}
              end={item.href === '/'}
              class="shell-nav-item"
              classList={{ unavailable: !navAvailable(item) }}
              aria-current={active(item) ? 'page' : undefined}
              title={`${item.label} (${modKey()}+${index() + 1})`}
            >
              {item.icon(18)}
              <span class="shell-nav-label">{item.label}</span>
              <Show when={!navAvailable(item)} fallback={<span class="shell-nav-key">{index() + 1}</span>}>
                <span class="shell-nav-app" title="Needs the UniPi app">app</span>
              </Show>
            </A>
          )}
        </For>
      </nav>
      <div class="shell-sidebar-foot">{props.footer}</div>
    </aside>
  );
}

/** The splitter between list and detail; drag to resize, double-click resets. */
function Splitter(props: { screen: string }) {
  let dragging = false;
  const onDown = (event: PointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    dragging = true;
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
    document.documentElement.dataset.resizing = '';
  };
  const onMove = (event: PointerEvent) => {
    if (!dragging) return;
    layout().setListWidth(props.screen, event.clientX - SIDEBAR_WIDTH);
  };
  const onUp = () => {
    dragging = false;
    delete document.documentElement.dataset.resizing;
  };
  const onKey = (event: KeyboardEvent) => {
    const step = event.shiftKey ? 64 : 16;
    if (event.key === 'ArrowLeft') layout().setListWidth(props.screen, layout().listWidth(props.screen) - step);
    else if (event.key === 'ArrowRight') layout().setListWidth(props.screen, layout().listWidth(props.screen) + step);
    else return;
    event.preventDefault();
  };
  return (
    <div
      class="shell-splitter"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize list"
      aria-valuenow={layout().listWidth(props.screen)}
      tabindex="0"
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
      onDblClick={() => layout().resetListWidth(props.screen)}
      onKeyDown={onKey}
      data-testid="shell-splitter"
    />
  );
}

/** Generic "nothing selected" filler for a detail pane. */
export function PanePlaceholder(props: { title: string; hint?: string; icon?: JSX.Element }) {
  // UNI-218 F10: the shared EmptyState (compact), same as list states.
  return <EmptyState compact icon={props.icon ?? <CommandIcon size={22} />} title={props.title} body={props.hint} testId="pane-placeholder" />;
}

/**
 * Desktop-only chrome + global wiring. Mounted once by the app shell; does
 * nothing (and sets `data-shell=phone`) below the desktop breakpoint or on
 * the phone app, so the phone layout is untouched.
 */
export function DesktopChrome() {
  const location = useLocation();
  const navigate = useNavigate();
  const desktop = () => layout().desktop();
  const screen = createMemo(() => screenOf(location.pathname));
  const pane = () => (desktop() ? paneFor(screen()) : undefined);
  const paneOpen = () => !!pane()?.open();
  const split = () => !!pane() && (paneOpen() || !!pane()!.placeholder);

  // UNI-312: a sidebar tab switch shows that tab's screen, not the chat
  // layer left over from before — and clicking Chat again brings it back
  // (draft/scroll kept, same connection). Desktop shell only: on the
  // phone the chat layer covers the bottom nav itself, so there is no tab
  // to switch under it (Back/return-to-chat there are unaffected).
  createEffect((prevScreen: string | undefined) => {
    const next = screen();
    if (desktop()) syncChatVisibleOnScreenChange(prevScreen, next);
    return next;
  }, undefined);

  createEffect(() => {
    const root = document.documentElement;
    root.dataset.shell = desktop() ? 'desktop' : 'phone';
    if (split()) {
      const list = layout().listWidth(screen());
      root.dataset.detail = paneOpen() ? 'open' : 'placeholder';
      root.style.setProperty('--list-w', `${list}px`);
      root.style.setProperty('--detail-left', `${SIDEBAR_WIDTH + list}px`);
    } else {
      delete root.dataset.detail;
      root.style.removeProperty('--list-w');
      root.style.setProperty('--detail-left', desktop() ? `${SIDEBAR_WIDTH}px` : '0px');
    }
  });
  onCleanup(() => {
    const root = document.documentElement;
    delete root.dataset.shell;
    delete root.dataset.detail;
  });

  // Window title: DesktopNative (follows the screen + the open chat/terminal/file).

  // "Go to" commands (registered first so they lead the palette) + global shortcuts.
  onCleanup(
      registerCommands([
        ...NAV.map((item, index) => ({
          id: `go.${item.screen}`,
          title: `Go to ${item.label}`,
          group: 'Go to',
          keywords: item.screen,
          keys: `${modKey()} ${index + 1}`,
          icon: () => item.icon(16),
          run: () => navigate(item.href),
        })),
        { id: 'app.shortcuts', title: 'Keyboard shortcuts', group: 'App', keywords: 'keys help hotkeys', keys: `${modKey()} /`, icon: () => <Keyboard size={16} />, run: () => setShortcutsOpen(true) },
      ]),
  );
  onMount(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!desktop()) return;
      const mod = event.ctrlKey || event.metaKey;
      if (!mod || event.altKey) return;
      if (event.key.toLowerCase() === 'k' && !event.shiftKey) {
        // Inside a terminal plain Ctrl+K is the shell's (kill line); Ctrl+Shift+K opens the palette there.
        if ((event.target as HTMLElement | null)?.closest?.('.term-layer')) return;
        event.preventDefault();
        setPaletteOpen(!paletteOpen());
        return;
      }
      if (event.key.toLowerCase() === 'k' && event.shiftKey) {
        event.preventDefault();
        setPaletteOpen(!paletteOpen());
        return;
      }
      if (event.key === '/') {
        event.preventDefault();
        setPaletteOpen(false);
        setShortcutsOpen(!shortcutsOpen());
        return;
      }
      if (/^[1-9]$/.test(event.key) && !event.shiftKey) {
        const item = NAV[Number(event.key) - 1];
        if (!item) return;
        event.preventDefault();
        setPaletteOpen(false);
        navigate(item.href);
      }
    };
    window.addEventListener('keydown', onKey);
    onCleanup(() => window.removeEventListener('keydown', onKey));
  });

  return (
    <Show when={split()}>
      <Splitter screen={screen()} />
      <Show when={!paneOpen() && pane()?.placeholder}>
        <div class="shell-placeholder">{pane()!.placeholder!()}</div>
      </Show>
    </Show>
  );
}
