/**
 * /term — full-screen terminal (DESIGN §3.3). The app shell mounts one
 * TerminalLayer; it keeps a TerminalScreen alive for every live
 * session (lib/liveSessions) and shows the active one while the route
 * is /term. Leaving (yellow "–", knob drag-down, Android Back) only
 * hides the screen: xterm, scrollback and the attach stream stay up
 * in the background. A screen is torn down (stream closed = detach,
 * the PC session keeps running) only when its live entry is closed.
 */
import { isTauri } from '../lib/platform';
import { desktopShell } from '../lib/layout';
import { nextFontSize, termKey } from '../lib/desktop';
import { createEffect, createSignal, For, on, onCleanup, onMount, Show, untrack } from 'solid-js';
import z from 'zod';
import { useLocation, useNavigate } from '@solidjs/router';
import { Terminal } from '@xterm/xterm';
import { WebglAddon } from '@xterm/addon-webgl';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { List, Minus, X } from 'lucide-solid';
import { TermSession } from '../lib/termSession';
import { call } from '../lib/ipc';
import { setTermTarget, type TermTarget } from '../lib/termTarget';
import {
  activeLive,
  activeLiveKey,
  closeLive,
  liveByRecency,
  liveStatus,
  liveEntries,
  registerLiveSnapshot,
  setLiveStatus,
  type LiveEntry,
} from '../lib/liveSessions';
import { knobRelease } from '../lib/knobDrag';
import { resumeChatAfterTerminal } from '../lib/chat/open';
import { computeFit } from '../lib/termFit';
import { termTheme } from '../lib/termTheme';
import { TERM_FONT_FAMILY, onLateFontLoad, termFontsReady } from '../lib/termFonts';
import { Reconnect, type ReconnectState } from '../lib/reconnect';
import { SessionPicker } from '../components/SessionPicker';
import { ErrorBanner } from '../components/ErrorBanner';
import { useSettings } from '../settings/context';
import { useHosts } from '../hosts/context';
import { useRecents } from '../lib/recentsContext';
import TermToolbar from '../components/TermToolbar';
import Composer, { HistorySheet } from '../components/Composer';
import ShortcutsSheet from '../components/ShortcutsSheet';
import { KEYDOWN_TO_KEY, Modifiers, bracketedPaste, composeSend, imeKeyBytes, kittyKeyBytes } from '../lib/termToolbar';
import { diffImeValue, imeInitial } from '../lib/ime';
import {
  LONG_PRESS_MS,
  SWIPE_THRESHOLD_PX,
  edgeStart,
  edgeSwipeDirection,
  isDoubleTap,
  pinchFontSize,
  twoFingerSwipe,
} from '../lib/gestures';
import {
  MOMENTUM_MIN_VELOCITY,
  MOMENTUM_START_VELOCITY,
  SCROLL_SLOP_PX,
  ScrollAccumulator,
  arrowSequence,
  cellAt,
  clickSequence,
  decayVelocity,
  releaseVelocity,
  scrollStepPx,
  scrollTargetFor,
  wheelSequence,
} from '../lib/termScroll';
import type { AppError } from '@unipi/protocol';
import type { TermEvent } from '../lib/term';
import type { TermSession as TermSessionType } from '../lib/termSession';

const RESIZE_DEBOUNCE_MS = 100;
/** Top-chrome collapsed state (global, UNI-128). */
const CHROME_KEY = 'unipi:term-chrome-collapsed';

type Ended =
  | { reason: 'exited' }
  | { reason: 'error'; error: AppError }
  | { reason: 'unreachable' }
  | undefined;

/**
 * Terminal layer (mounted once by the app shell, above the routes):
 * one TerminalScreen per live session, in insertion order so a screen
 * is never remounted while its entry lives. Only the active entry is
 * shown, and only on /term; the others stay connected, hidden. The
 * /term route itself renders Home underneath, so dragging the knob
 * down reveals the real dashboard.
 */
export default function TerminalLayer() {
  const location = useLocation();
  const navigate = useNavigate();
  const onTerm = () => location.pathname === '/term';
  createEffect(() => {
    // /term without an active session (cold deep link, closed entry).
    if (onTerm() && !activeLive()) navigate('/', { replace: true });
  });
  // Opened from a chat's Terminal switch: leaving the terminal returns to
  // that chat (UNI-113). Any other route change clears the link.
  createEffect(
    on(onTerm, (now, was) => {
      if (was && !now) resumeChatAfterTerminal();
    }),
  );
  return (
    <>
      {/* UNI-117 desktop: every live connection as a tab over the terminal pane. */}
      <Show when={desktopShell() && onTerm() && liveEntries().length > 0}>
        <TermTabs />
      </Show>
      <For each={liveEntries()}>
        {(entry) => {
          const visible = () => onTerm() && activeLiveKey() === entry.key;
          return (
            <div class="term-layer" hidden={!visible()} data-live-key={entry.key}>
              <TerminalScreen entry={entry} visible={visible()} />
            </div>
          );
        }}
      </For>
    </>
  );
}

/** Desktop tab strip: the live terminals (switch = resume, ✕ = detach; the session keeps running on the PC). */
function TermTabs() {
  const navigate = useNavigate();
  const hosts = useHosts();
  const hostName = (id: string) => hosts.hosts().find((h) => h.id === id)?.name ?? id;
  const close = (key: string) => {
    const others = liveByRecency().filter((e) => e.key !== key);
    closeLive(key);
    const next = others[0];
    if (next) setTermTarget(next.target);
    else navigate('/', { replace: true });
  };
  return (
    <div class="term-tabs" role="tablist" aria-label="Open terminals">
      <For each={liveEntries()}>
        {(entry) => (
          <div class="term-tab" role="tab" aria-selected={activeLiveKey() === entry.key} data-status={liveStatus(entry.key)}>
            <button type="button" class="term-tab-main" title={`${hostName(entry.target.hostId)}: ${entry.target.label}`} onClick={() => setTermTarget(entry.target)}>
              <span class="term-tab-dot" />
              <span class="term-tab-label truncate">{entry.target.label}</span>
              <span class="term-tab-host truncate">{hostName(entry.target.hostId)}</span>
            </button>
            <button type="button" class="term-tab-close" aria-label={`Close ${entry.target.label}`} onClick={() => close(entry.key)}>
              <X size={13} />
            </button>
          </div>
        )}
      </For>
    </div>
  );
}

function TerminalScreen(props: { entry: LiveEntry; visible: boolean }) {
  const navigate = useNavigate();
  const [settings] = useSettings();
  const hosts = useHosts();
  const recents = useRecents();
  // eslint-disable-next-line solid/reactivity -- one screen per live entry
  const target: TermTarget | null = props.entry.target;
  // eslint-disable-next-line solid/reactivity -- stable per entry
  const liveKey = props.entry.key;
  const [path, setPath] = createSignal<'lan' | 'tailnet' | 'direct' | 'relay' | 'local' | 'unknown'>('unknown');
  const [reconnectState, setReconnectState] = createSignal<ReconnectState>({ kind: 'idle' });
  const [banner, setBannerSignal] = createSignal<string | undefined>(undefined);
  const [reconnecting, setReconnecting] = createSignal(false);
  const [attempt, setAttempt] = createSignal(0);
  const [countdown, setCountdown] = createSignal<number | undefined>(undefined);
  const [ended, setEnded] = createSignal<Ended>(undefined);
  const [collapsed, setCollapsed] = createSignal(chromeCollapsedStored());
  const setChromeCollapsed = (value: boolean) => {
    setCollapsed(value);
    try {
      localStorage.setItem(CHROME_KEY, String(value));
    } catch {
      // storage unavailable
    }
  };
  /** Slim Moshi-style top chrome for every target (UNI-128): knob,
   * exit, session picker, `host: session` title, route badge. herdr
   * draws its own header right below it (taps reach it as mouse
   * clicks). Collapsing leaves only the knob strip. */
  const chromeHidden = () => collapsed();
  const [pickerOpen, setPickerOpen] = createSignal(false);
  const [keyboardOpen, setKeyboardOpen] = createSignal(false);
  const [composerOpen, setComposerOpen] = createSignal(false);
  const [shortcutsOpen, setShortcutsOpen] = createSignal(false);
  const [historyOpen, setHistoryOpen] = createSignal(false);
  const [selectionMode, setSelectionMode] = createSignal(false);
  const [, updateSettings] = useSettings();
  const modifiers = new Modifiers();
  let container: HTMLDivElement | undefined;
  let termHolder: HTMLDivElement | undefined;
  let screenEl: HTMLDivElement | undefined;
  let chromeEl: HTMLElement | undefined;
  /** Set once the screen is unmounted (live entry closed). */
  let disposed = false;
  let term: Terminal | undefined;

  let session: TermSessionType | undefined;
  let openedOnce = false;
  let recentWritten = false;
  let reconnect: Reconnect | undefined;
  let resizeTimer: ReturnType<typeof setTimeout> | undefined;
  let countdownTimer: ReturnType<typeof setInterval> | undefined;
  let imeState = imeInitial();
  let imeElRef: HTMLTextAreaElement | undefined;

  /** Collapsed-chrome memory: expanded unless the user collapsed it. */
  function chromeCollapsedStored(): boolean {
    try {
      return localStorage.getItem(CHROME_KEY) === 'true';
    } catch {
      return false;
    }
  }

  /** Leaves the terminal for Home. /term is normally entered from
   * Home, so this is a history back (Android Back does the same);
   * with no history (deep link) it replaces the route instead. */
  function goHome(): void {
    if (window.history.length > 1) window.history.back();
    else navigate('/', { replace: true });
  }

  // --- background / foreground (live sessions) ---
  const shownWaiters: (() => void)[] = [];
  /** Resolves once the screen is visible (immediately if it is). */
  function shown(): Promise<void> {
    if (untrack(() => props.visible)) return Promise.resolve();
    return new Promise((resolve) => shownWaiters.push(resolve));
  }
  createEffect(
    on(
      () => props.visible,
      (visible) => {
        if (visible) {
          for (const resolve of shownWaiters.splice(0)) resolve();
          resetDrag();
          // Back from the background: the holder was display:none, so
          // re-measure, repaint and refit (the PTY may need a resize).
          scheduleFit();
          term?.refresh(0, Math.max(0, term.rows - 1));
          // Desktop: typing goes straight to the shown terminal.
          if (desktopShell()) queueMicrotask(() => term?.focus());
          return;
        }
        // Sent to the background: drop transient UI, keep the stream.
        setPickerOpen(false);
        setComposerOpen(false);
        setShortcutsOpen(false);
        setHistoryOpen(false);
        setSelectionMode(false);
        term?.clearSelection();
        stopMomentum();
        imeElRef?.blur();
        resetDrag();
      },
      { defer: true },
    ),
  );

  /** Visible text of the terminal (dashboard preview for shells). */
  function readSnapshot(): string | undefined {
    const t = term;
    if (!t) return undefined;
    const buffer = t.buffer.active;
    const lines: string[] = [];
    const start = buffer.viewportY;
    for (let i = 0; i < t.rows; i += 1) {
      lines.push(buffer.getLine(start + i)?.translateToString(true) ?? '');
    }
    while (lines.length > 0 && lines[lines.length - 1].trim() === '') lines.pop();
    return lines.join('\n');
  }

  /** xterm + addons + input listeners: created exactly once. */
  function initTerminal() {
    const t = new Terminal({
      allowProposedApi: true,
      theme: { ...termTheme, background: '#141211' },
      fontFamily: TERM_FONT_FAMILY,
      fontSize: settings.terminal.fontSize,
      lineHeight: settings.terminal.lineHeight,
      cursorStyle: settings.terminal.cursorStyle,
      cursorBlink: settings.terminal.cursorBlink,
      scrollback: settings.terminal.scrollback,
    });
    term = t;
    const unicode11 = new Unicode11Addon();
    t.loadAddon(unicode11);
    t.unicode.activeVersion = '11';
    t.loadAddon(new WebLinksAddon());
    t.open(termHolder!);
    // UNI-313: xterm.js already sets autocorrect/autocapitalize/spellcheck
    // off on its own hidden textarea, but not autocomplete — WebKitGTK
    // (the desktop webview) can still offer/insert a saved-value
    // completion on that field, which showed up as a stray trailing
    // space while typing. Belt-and-suspenders: set all four.
    if (t.textarea) {
      t.textarea.setAttribute('autocomplete', 'off');
      t.textarea.setAttribute('autocorrect', 'off');
      t.textarea.setAttribute('autocapitalize', 'off');
      t.textarea.setAttribute('spellcheck', 'false');
    }
    try {
      // Headless capture browsers render nothing with WebGL; the
      // capture harness opts out through this flag.
      if (!('__UNIPI_NO_WEBGL__' in window)) {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => webgl.dispose());
        t.loadAddon(webgl);
      }
    } catch {
      // DOM renderer fallback
    }

    // xterm's onBinary: a latin1 string where each char is a byte.
    t.onBinary((data) => session?.inputBinary(data));
    // UNI-117 desktop: a physical keyboard + mouse talk to xterm directly
    // (its own key encoding, mouse reports, wheel, Shift+drag selection);
    // the phone's IME textarea, toolbar and touch gestures stay out.
    t.onData((data) => {
      if (desktopShell()) session?.input(data);
    });
    t.attachCustomKeyEventHandler((event) => (desktopShell() ? desktopKey(event) : true));
    wireIme();
    // xterm focuses its own hidden helper textarea on mousedown
    // (for native text selection); keep input exclusively on
    // #term-ime by bouncing focus back whenever that happens.
    t.textarea?.addEventListener('focus', () => {
      if (desktopShell()) return;
      if (document.activeElement === t.textarea) imeElRef?.focus();
    });
  }

  /**
   * Desktop keys xterm must not send (lib/desktop.ts termKey); false =
   * handled here. App shortcuts (copy/paste/font/palette/Ctrl+1…9) win
   * first; anything left that xterm would otherwise eat or mis-encode
   * (Shift+Enter, Ctrl+Enter, Ctrl+Shift+<letter>…) goes out as a
   * kitty-style CSI-u sequence (UNI-311, lib/termToolbar.kittyKeyBytes)
   * instead of falling through to xterm's own, narrower encoder.
   */
  function desktopKey(event: KeyboardEvent): boolean {
    const action = termKey(event, /Mac/.test(navigator.platform || ''));
    // Esc is the shell's (vim, pi…), not the app's Back.
    if (event.key === 'Escape') event.stopPropagation();
    if (action) {
      if (event.type !== 'keydown') return false;
      if (action.kind === 'app') {
        // Not sent to the shell; the event bubbles on to the app's own handler
        // (DesktopChrome: Ctrl+Shift+K palette, Ctrl+1…9, ⌘ combos). Opening the
        // palette here as well made that handler's toggle close it again at once.
        return false;
      }
      event.preventDefault();
      if (action.kind === 'copy') {
        const selected = term?.getSelection();
        if (selected) void clipboardWrite(selected);
      } else if (action.kind === 'paste') {
        void pasteClipboard();
      } else if (action.kind === 'font' && term) {
        const next = nextFontSize(term.options.fontSize ?? 14, action.delta);
        if (term.options.fontSize !== next) {
          term.options.fontSize = next;
          updateSettings('terminal.fontSize', next);
          scheduleFit();
        }
      }
      return false;
    }
    if (event.type !== 'keydown') return true;
    const bytes = kittyKeyBytes(event);
    if (bytes !== null) {
      event.preventDefault();
      session?.input(bytes);
      return false;
    }
    return true;
  }

  /**
   * Wires `#term-ime`: the single source of truth for soft-keyboard
   * text (DESIGN §3.3, lib/ime). `input` diffs the textarea's value
   * against the last value we sent bytes for — this works
   * identically for GBoard composition, HyperOS autocorrect rewrites
   * and `adb shell input text`, because it never interprets event
   * semantics, only the resulting text. `keydown` only handles
   * non-printable keys (Enter/Backspace/arrows/Esc/Tab/Ctrl combos);
   * printable keydowns are intentionally NOT sent here — the `input`
   * diff owns them, so each character is sent exactly once. xterm's
   * own helper textarea is kept out of the loop entirely: a plain tap
   * focuses `#term-ime` (see gestureUp; drags/scrolls don't, so
   * scrolling never pops the keyboard).
   */
  function wireIme(): void {
    const ime = imeElRef;
    if (!ime) return;
    ime.addEventListener('input', () => {
      const result = diffImeValue(ime.value, imeState);
      imeState = result.state;
      if (result.bytes) {
        // Armed toolbar modifiers apply to soft-keyboard text too
        // (Ctrl one-shot + GBoard `c` → \x03).
        const modified = modifiers.applyToText(result.bytes);
        session?.input(modified.bytes);
        if (modified.applied) {
          // The textarea now holds text the PTY never saw verbatim:
          // start a fresh diff so autocorrect can't "fix" it later.
          ime.value = '';
          imeState = imeInitial();
          return;
        }
      }
      if (result.resetValue !== undefined) ime.value = result.resetValue;
    });
    ime.addEventListener('keydown', (event) => {
      // Printable, non-modified keys: let the native textarea update
      // its value; the `input` listener above diffs and sends it.
      if (event.key.length === 1 && !event.ctrlKey && !event.altKey && !event.metaKey) return;
      const named = KEYDOWN_TO_KEY[event.key];
      if (named && modifiers.anyActive() && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        session?.input(modifiers.consume(named));
        return;
      }
      const bytes = imeKeyBytes(event.key, event.ctrlKey, event.altKey);
      if (!bytes) return;
      event.preventDefault();
      session?.input(bytes);
    });
  }

  function onTermEvent(event: TermEvent) {
    if (event.type === 'title') return;
    reconnect?.dropped(event);
  }

  /** Creates a new TermSession and opens it. Resets the terminal
   * first when this is a reconnect (herdr and tmux repaint on
   * attach), never on the first open. Never throws. */
  async function connect() {
    if (!target) return;
    await untrack(async () => {
      if (session) {
        await session.close();
        session = undefined;
      }
      // On reconnects herdr and tmux repaint on attach; clear the
      // stale buffer before the new stream writes. Never on the very
      // first open (nothing to clear, and it would wipe nothing).
      if (openedOnce) {
        term?.reset();
      }
      const cols = term?.cols ?? 80;
      const rows = term?.rows ?? 24;
      session = new TermSession(
        {
          hostId: target.hostId,
          relay: untrack(() => settings.connection.relay),
          target: target.target,
          cols,
          rows,
          sizePolicy: untrack(() => settings.terminal.sizePolicy),
        },
        {
          onData: (bytes) => term?.write(bytes),
          onEvent: onTermEvent,
        },
      );
      const opening = session;
      const result = await opening.open();
      if (disposed) {
        // The connection was closed while it was still opening.
        void opening.close();
        return;
      }
      openedOnce = true;
      if (result.ok) {
        setPath(result.value.path);
        hosts.setPath(target.hostId, result.value.path);
        // Record the recent once per target: the first successful
        // open. Reconnects reuse the same session key and would only
        // bump it to the top, so they don't re-add.
        if (!recentWritten) {
          recentWritten = true;
          recents.addRecent({
            hostId: target.hostId,
            target: target.target,
            label: target.label,
            sublabel: target.sublabel,
            openedAt: Date.now(),
          });
        }
        reconnect?.live();
        scheduleFit();
      } else {
        reconnect?.dropped({ type: 'closed', error: result.error });
      }
    });
  }

  function startSession() {
    if (!target) return;
    setEnded(undefined);
    setCountdown(undefined);
    reconnect = new Reconnect({
      onRetry: () => void connect(),
      onState: (state) => onReconnectState(state),
    });
    reconnect.beginConnecting();
    void connect();
  }

  function onReconnectState(state: ReconnectState) {
    if (disposed) return;
    setLiveStatus(
      liveKey,
      state.kind === 'live'
        ? 'live'
        : state.kind === 'reconnecting'
          ? 'reconnecting'
          : state.kind === 'ended'
            ? 'ended'
            : 'connecting',
    );
    const hostName = target?.label ?? 'host';
    if (state.kind === 'live') {
      setCountdown(undefined);
      setBannerSignal(undefined);
      setReconnecting(false);
      setEnded(undefined);
      return;
    }
    if (state.kind === 'reconnecting') {
      setReconnecting(true);
      setAttempt(state.attempt);
      setBannerSignal(`Reconnecting via ${pathLabel()}…`);
      setCountdown(Math.ceil(state.nextDelayMs / 1000));
      reconnect?.armTimer();
      return;
    }
    if (state.kind === 'connecting') {
      setReconnecting(true);
      setBannerSignal(`Connecting to ${hostName}…`);
      return;
    }
    if (state.kind === 'ended') {
      setCountdown(undefined);
      setBannerSignal(undefined);
      if (state.reason === 'exited') {
        setEnded({ reason: 'exited' });
      } else if (state.reason === 'unreachable') {
        setEnded({
          reason: 'error',
          error: {
            code: 'E_TIMEOUT',
            message: `Couldn't reach ${hostName} for 2 minutes`,
            retryable: true,
          },
        });
      } else {
        setEnded({ reason: 'error', error: state.error! });
      }
    }
  }

  function pathLabel(): string {
    switch (path()) {
      case 'lan':
        return 'LAN';
      case 'tailnet':
        return 'Tailnet';
      case 'direct':
        return 'Direct';
      case 'relay':
        return 'Relay';
      default:
        return 'last route';
    }
  }

  onMount(() => {
    if (!target) {
      navigate('/', { replace: true });
      return;
    }
    // Open xterm only once its fonts are in (it measures the cell at
    // open) and the screen is shown (a display:none holder measures
    // 0); a face that lands later re-measures and refits.
    void (async () => {
      await termFontsReady(settings.terminal.fontSize);
      await shown();
      if (disposed) return;
      initTerminal();
      startSession();
      scheduleFit();
      if (desktopShell()) term?.focus();
    })();
    const stopLateFonts = onLateFontLoad(() => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (term as any)?._core?._charSizeService?.measure?.();
      term?.refresh(0, (term?.rows ?? 1) - 1);
      scheduleFit();
    });

    const observer = new ResizeObserver(() => scheduleFit());
    if (container) observer.observe(container);
    container?.addEventListener('mousedown', blockCompatMouse, true);
    container?.addEventListener('mouseup', blockCompatMouse, true);
    const onViewport = () => {
      // Background screens leave the shared keyboard inset alone.
      if (!props.visible) return;
      scheduleFit();
      const keyboard = Math.max(
        0,
        window.innerHeight - (window.visualViewport?.height ?? window.innerHeight),
        // Native keyboard inset (Android, adjustResize), set by MainActivity.
        (window as unknown as { unipiImeInset?: number }).unipiImeInset ?? 0,
      );
      document.documentElement.style.setProperty('--inset-ime', `${Math.round(keyboard)}px`);
      setKeyboardOpen(keyboard > 120);
    };
    window.visualViewport?.addEventListener('resize', onViewport);
    window.addEventListener('unipi-ime', onViewport);
    const onVisible = () => {
      if (!document.hidden && reconnect?.state.kind === 'reconnecting') {
        reconnect.fireRetry();
      }
    };
    document.addEventListener('visibilitychange', onVisible);
    countdownTimer = setInterval(() => {
      const remaining = countdown();
      if (remaining !== undefined && remaining > 0) setCountdown(remaining - 1);
    }, 1000);
    const stopSnapshot = registerLiveSnapshot(liveKey, readSnapshot);
    onCleanup(() => {
      disposed = true;
      stopSnapshot();
      reconnect?.dispose();
      for (const resolve of shownWaiters.splice(0)) resolve();
      stopLateFonts();
      observer.disconnect();
      container?.removeEventListener('mousedown', blockCompatMouse, true);
      container?.removeEventListener('mouseup', blockCompatMouse, true);
      window.visualViewport?.removeEventListener('resize', onViewport);
      window.removeEventListener('unipi-ime', onViewport);
      clearTimeout(longPressTimer);
      stopMomentum();
      document.removeEventListener('visibilitychange', onVisible);
      if (countdownTimer) clearInterval(countdownTimer);
      if (resizeTimer) clearTimeout(resizeTimer);
      void session?.close();
      term?.dispose();
    });
  });

  function relayArg(): string {
    const relay = untrack(() => settings.connection.relay);
    return relay?.mode === 'custom' ? relay.url : 'n0';
  }

  async function clipboardRead(): Promise<string | undefined> {
    try {
      if (isTauri()) {
        const clipboard = await import('@tauri-apps/plugin-clipboard-manager');
        return await clipboard.readText();
      }
    } catch {
      // fall through to the web API
    }
    try {
      return await navigator.clipboard.readText();
    } catch {
      return undefined;
    }
  }

  async function clipboardWrite(text: string): Promise<void> {
    try {
      if (isTauri()) {
        const clipboard = await import('@tauri-apps/plugin-clipboard-manager');
        await clipboard.writeText(text);
        return;
      }
    } catch {
      // fall through to the web API
    }
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Nothing more we can do.
    }
  }

  /** Applies a mux_action for the current target's multiplexer. */
  async function muxAction(action: Record<string, unknown>): Promise<boolean> {
    if (!target?.target) return false;
    const result = await call(
      'mux_action',
      { hostId: target.hostId, relay: relayArg(), action },
      z.unknown(),
    );
    return result.ok;
  }

  /** Pane switching: two-finger horizontal swipe. */
  function switchPane(direction: 'pane_next' | 'pane_prev'): void {
    const mux = target?.target?.mux;
    const session = target?.target?.session;
    if (!mux || !session) return;
    if (mux === 'herdr') void muxAction({ action: direction, session });
    if (mux === 'tmux') {
      void muxAction({
        action: direction === 'pane_next' ? 'tmux_pane_next' : 'tmux_pane_prev',
        session,
      });
    }
  }

  /** Tab/window switching: one-finger edge swipe. */
  function switchTabWindow(direction: 'tab_next' | 'tab_prev'): void {
    const mux = target?.target?.mux;
    const session = target?.target?.session;
    if (!mux || !session) return;
    if (mux === 'herdr') void muxAction({ action: direction, session });
    if (mux === 'tmux') {
      void muxAction({
        action: direction === 'tab_next' ? 'window_next' : 'window_prev',
        session,
      });
    }
  }

  async function pasteClipboard(): Promise<void> {
    const text = await clipboardRead();
    if (text) session?.input(bracketedPaste(text));
  }

  /** Keyboard key: closes the soft keyboard when it is up, opens it
   * otherwise. Android only shows the IME on a focus *change*, so a
   * still-focused textarea (keyboard dismissed with Back) is blurred
   * and refocused inside the same tap. */
  function toggleKeyboard(): void {
    const ime = imeElRef;
    if (!ime) return;
    if (keyboardOpen()) {
      ime.blur();
      return;
    }
    if (document.activeElement === ime) ime.blur();
    ime.focus();
  }

  // --- gesture plumbing over the terminal surface ---
  const pointers = new Map<number, { x: number; y: number }>();
  let tapTimes: number[] = [];
  let tapPosition = { x: 0, y: 0 };
  let twoFingerStart: number | undefined;
  let twoFingerFired = false;
  let pinchBaseDistance: number | undefined;
  let pinchBaseSize: number | undefined;
  let longPressTimer: ReturnType<typeof setTimeout> | undefined;
  let edgeDrag: { edge: 'left' | 'right'; startX: number } | undefined;
  // One-finger vertical scroll (lib/termScroll).
  let scrolling = false;
  let gestureMoved = false;
  let multiTouch = false;
  let scrollLastY = 0;
  let scrollAnchor = { x: 0, y: 0 };
  let scrollSamples: { t: number; y: number }[] = [];
  const scrollAcc = new ScrollAccumulator(48);
  let momentumFrame: number | undefined;
  let imeFocusedAtDown = false;

  function cellSize(): { width: number; height: number } | undefined {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const cell = (term as any)?._core?._renderService?.dimensions?.css?.cell;
    return cell && cell.width > 0 && cell.height > 0 ? cell : undefined;
  }

  /** What a vertical drag drives right now, and how far one step is. */
  function scrollPlan() {
    const t = term;
    const cell = cellSize();
    if (!t || !cell) return undefined;
    const kind = scrollTargetFor(t.modes.mouseTrackingMode, t.buffer.active.type);
    const stepPx = scrollStepPx(kind, cell.height, target?.target?.mux);
    return { t, cell, kind, stepPx };
  }

  /** Emits `steps` scroll steps (positive = earlier content). */
  function emitScroll(steps: number): void {
    const plan = scrollPlan();
    if (!plan || steps === 0) return;
    const direction = steps > 0 ? 'up' : 'down';
    const count = Math.min(Math.abs(steps), 40);
    if (plan.kind === 'scrollback') {
      plan.t.scrollLines(-steps);
      return;
    }
    let bytes = '';
    if (plan.kind === 'mouse') {
      const screen = plan.t.element?.querySelector('.xterm-screen')?.getBoundingClientRect();
      const origin = screen ? { left: screen.left, top: screen.top } : { left: 0, top: 0 };
      const { col, row } = cellAt(scrollAnchor, origin, plan.cell, { cols: plan.t.cols, rows: plan.t.rows });
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const encoding = (plan.t as any)._core?.coreMouseService?.activeEncoding ?? 'SGR';
      const pixel = { x: scrollAnchor.x - origin.left, y: scrollAnchor.y - origin.top };
      for (let i = 0; i < count; i += 1) bytes += wheelSequence(direction, col, row, encoding, pixel);
    } else {
      const app = plan.t.modes.applicationCursorKeysMode;
      for (let i = 0; i < count; i += 1) bytes += arrowSequence(direction, app);
    }
    session?.input(bytes);
  }

  function scrollBy(dy: number): void {
    const plan = scrollPlan();
    if (!plan) return;
    scrollAcc.setStep(plan.stepPx);
    emitScroll(scrollAcc.push(dy));
  }

  function stopMomentum(): void {
    if (momentumFrame !== undefined) cancelAnimationFrame(momentumFrame);
    momentumFrame = undefined;
  }

  function startMomentum(initial: number): void {
    stopMomentum();
    if (Math.abs(initial) < MOMENTUM_START_VELOCITY) return;
    if (window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;
    let velocity = initial;
    let last = performance.now();
    const frame = (now: number) => {
      const dt = Math.min(48, now - last);
      last = now;
      scrollBy(velocity * dt);
      velocity = decayVelocity(velocity, dt);
      momentumFrame = Math.abs(velocity) < MOMENTUM_MIN_VELOCITY ? undefined : requestAnimationFrame(frame);
    };
    momentumFrame = requestAnimationFrame(frame);
  }

  /** Sends a left click (press + release) at a client point, encoded
   * the way the app asked for (SGR / SGR-pixels / X10). */
  function sendClick(x: number, y: number): void {
    const t = term;
    const cell = cellSize();
    if (!t || !cell) return;
    const screen = t.element?.querySelector('.xterm-screen')?.getBoundingClientRect();
    const origin = screen ? { left: screen.left, top: screen.top } : { left: 0, top: 0 };
    const { col, row } = cellAt({ x, y }, origin, cell, { cols: t.cols, rows: t.rows });
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const encoding = (t as any)._core?.coreMouseService?.activeEncoding ?? 'SGR';
    const pixel = { x: x - origin.left, y: y - origin.top };
    const bytes = clickSequence(col, row, encoding, pixel, t.modes.mouseTrackingMode !== 'x10');
    // X10 ("DEFAULT") reports are raw bytes ≥ 0x80 for far cells.
    if (encoding === 'DEFAULT') session?.inputBinary(bytes);
    else session?.input(bytes);
  }

  /**
   * Keeps xterm's own mouse handling out of touch taps while the app
   * tracks the mouse: the compat mousedown that follows a tap would
   * make xterm emit its own report (SGR via the unwired onData, X10
   * via onBinary → a double click) and move focus off `#term-ime`.
   * We encode the click ourselves in gestureUp.
   */
  function blockCompatMouse(event: MouseEvent): void {
    if (desktopShell()) return;
    if (!term || term.modes.mouseTrackingMode === 'none') return;
    event.stopPropagation();
    event.preventDefault();
  }

  function gestureDown(event: PointerEvent) {
    // Desktop: the mouse goes to xterm itself (clicks, selection, wheel).
    if (desktopShell() && event.pointerType === 'mouse') {
      if (event.button === 0) queueMicrotask(() => term?.focus());
      return;
    }
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.size === 1) {
      imeFocusedAtDown = document.activeElement === imeElRef;
      stopMomentum();
      scrolling = false;
      gestureMoved = false;
      multiTouch = false;
      scrollAcc.reset();
      scrollLastY = event.clientY;
      scrollAnchor = { x: event.clientX, y: event.clientY };
      scrollSamples = [{ t: event.timeStamp, y: event.clientY }];
      tapTimes = [...tapTimes.slice(-1), Date.now()];
      tapPosition = { x: event.clientX, y: event.clientY };
      const edge = edgeStart(event.clientX, window.innerWidth);
      if (edge) edgeDrag = { edge, startX: event.clientX };
      clearTimeout(longPressTimer);
      longPressTimer = setTimeout(() => {
        if (pointers.size === 1) setSelectionMode(true);
      }, LONG_PRESS_MS);
      return;
    }
    clearTimeout(longPressTimer);
    multiTouch = true;
    scrolling = false;
    if (pointers.size === 2) {
      twoFingerStart = midX();
      twoFingerFired = false;
      const [a, b] = [...pointers.values()];
      pinchBaseDistance = Math.abs(a.x - b.x) + Math.abs(a.y - b.y);
      pinchBaseSize = term?.options.fontSize as number | undefined;
    }
  }

  function midX(): number {
    const values = [...pointers.values()];
    return values.reduce((sum, p) => sum + p.x, 0) / Math.max(1, values.length);
  }

  function gestureMove(event: PointerEvent) {
    if (desktopShell() && event.pointerType === 'mouse') return;
    const previous = pointers.get(event.pointerId);
    if (!previous) return;
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.size === 1) {
      const moved =
        Math.abs(event.clientX - tapPosition.x) + Math.abs(event.clientY - tapPosition.y);
      if (moved > 10) {
        clearTimeout(longPressTimer);
        gestureMoved = true;
      }
      if (!scrolling && !selectionMode() && !multiTouch) {
        const dx = event.clientX - tapPosition.x;
        const dy = event.clientY - tapPosition.y;
        if (Math.abs(dy) > SCROLL_SLOP_PX && Math.abs(dy) > Math.abs(dx)) {
          // Vertical drag: scroll (and never a tap / edge swipe).
          scrolling = true;
          edgeDrag = undefined;
          tapTimes = [];
          scrollLastY = event.clientY;
        }
      }
      if (scrolling) {
        scrollBy(event.clientY - scrollLastY);
        scrollLastY = event.clientY;
        scrollSamples = [...scrollSamples.slice(-6), { t: event.timeStamp, y: event.clientY }];
        return;
      }
      if (edgeDrag) {
        if (chromeHidden()) {
          // Minimal chrome: left-edge swipe back goes Home.
          if (Math.abs(event.clientX - edgeDrag.startX) > SWIPE_THRESHOLD_PX) {
            edgeDrag = undefined;
            goHome();
          }
          return;
        }
        const direction = edgeSwipeDirection(
          edgeDrag.edge,
          event.clientX - edgeDrag.startX,
        );
        if (direction) {
          switchTabWindow(direction);
          edgeDrag = undefined;
        }
      }
      return;
    }
    if (pointers.size === 2) {
      if (twoFingerStart !== undefined && !twoFingerFired) {
        const swipe = twoFingerSwipe({ startX: twoFingerStart, currentX: midX() });
        if (swipe) {
          twoFingerFired = true;
          switchPane(swipe);
        }
      }
      const [a, b] = [...pointers.values()];
      const distance = Math.abs(a.x - b.x) + Math.abs(a.y - b.y);
      if (pinchBaseDistance && pinchBaseSize && distance > 40) {
        const next = pinchFontSize(pinchBaseSize, distance / pinchBaseDistance);
        if (term && term.options.fontSize !== next) {
          term.options.fontSize = next;
          updateSettings('terminal.fontSize', next);
          scheduleFit();
        }
      }
    }
  }

  function gestureUp(event: PointerEvent) {
    if (desktopShell() && event.pointerType === 'mouse') return;
    pointers.delete(event.pointerId);
    clearTimeout(longPressTimer);
    if (pointers.size < 2) {
      twoFingerStart = undefined;
      pinchBaseDistance = undefined;
      pinchBaseSize = undefined;
    }
    if (pointers.size === 0) {
      edgeDrag = undefined;
      if (scrolling) {
        scrolling = false;
        startMomentum(releaseVelocity(scrollSamples));
        return;
      }
      if (gestureMoved || multiTouch) {
        tapTimes = [];
        return;
      }
      if (term && term.modes.mouseTrackingMode !== 'none') {
        // The app tracks the mouse (herdr, tmux `mouse on`): a tap is
        // a click at that cell, so herdr's header ("switch", tabs,
        // close ×) and panes work by touch. The keyboard stays as it
        // was; the toolbar's keyboard key opens it.
        if (!selectionMode()) sendClick(event.clientX, event.clientY);
        if (imeFocusedAtDown) imeElRef?.focus();
      } else {
        // A plain tap focuses the IME (opens the soft keyboard).
        imeElRef?.focus();
      }
      // Double-tap paste (ignores the taps that made up gestures).
      if (!selectionMode() && isDoubleTap(tapTimes) && target?.target) {
        void pasteClipboard();
      }
    }
  }

  /** No horizontal gutter (DESIGN §3.3, UNI-128 follow-up): the grid
   * runs edge to edge; only the sub-cell pixel remainder is split
   * evenly left/right, painted in the terminal background. No
   * scrollbar/overview-ruler gutter is reserved (ours is hidden). */
  const BASE_PADDING_X = 0;

  function applyFit(): void {
    if (!term || !termHolder) return;
    // Hidden (background) screens measure 0: never shrink the PTY.
    if (!props.visible || termHolder.clientWidth === 0 || termHolder.clientHeight === 0) return;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const core = (term as any)._core;
    const dims = core?._renderService?.dimensions?.css?.cell;
    if (!dims || dims.width <= 0 || dims.height <= 0) return;
    const result = computeFit({
      containerWidth: termHolder.clientWidth,
      containerHeight: termHolder.clientHeight,
      cellWidth: dims.width,
      cellHeight: dims.height,
      basePaddingX: BASE_PADDING_X,
      basePaddingY: 0,
    });
    if (!result) return;
    term.element!.style.paddingLeft = `${result.paddingX}px`;
    term.element!.style.paddingRight = `${result.paddingX}px`;
    const before = { cols: term.cols, rows: term.rows };
    if (result.cols !== before.cols || result.rows !== before.rows) {
      core._renderService.clear();
      term.resize(result.cols, result.rows);
      session?.resize(term.cols, term.rows);
    }
  }

  function scheduleFit() {
    if (resizeTimer) clearTimeout(resizeTimer);
    resizeTimer = setTimeout(applyFit, RESIZE_DEBOUNCE_MS);
  }

  // --- knob drag-down → background (Moshi) ---
  // A vertical drag that starts on the top chrome moves the whole
  // terminal screen with the finger; on release it either slides away
  // (far or fast enough → background + Home) or snaps back. A short
  // upward swipe collapses the chrome. Taps on the knob / buttons are
  // untouched (the drag only engages after DRAG_SLOP_PX).
  const DRAG_SLOP_PX = 8;
  const [dragY, setDragY] = createSignal(0);
  const [dragTransition, setDragTransition] = createSignal<string | undefined>(undefined);
  let drag:
    | { id: number; x: number; y: number; engaged: boolean; samples: { t: number; y: number }[] }
    | undefined;
  let suppressClickUntil = 0;
  let dismissTimer: ReturnType<typeof setTimeout> | undefined;

  function resetDrag(): void {
    drag = undefined;
    clearTimeout(dismissTimer);
    setDragTransition(undefined);
    setDragY(0);
  }

  function chromeDown(event: PointerEvent): void {
    if (event.button > 0 || drag) return;
    drag = { id: event.pointerId, x: event.clientX, y: event.clientY, engaged: false, samples: [{ t: event.timeStamp, y: event.clientY }] };
  }

  function chromeMove(event: PointerEvent): void {
    if (!drag || event.pointerId !== drag.id) return;
    const dy = event.clientY - drag.y;
    const dx = event.clientX - drag.x;
    if (!drag.engaged) {
      if (dy <= DRAG_SLOP_PX || Math.abs(dy) <= Math.abs(dx)) return;
      drag.engaged = true;
      try {
        chromeEl?.setPointerCapture(event.pointerId);
      } catch {
        // pointer already gone
      }
      setDragTransition('none');
    }
    drag.samples = [...drag.samples.slice(-5), { t: event.timeStamp, y: event.clientY }];
    setDragY(Math.max(0, dy - DRAG_SLOP_PX));
  }

  function chromeUp(event: PointerEvent): void {
    if (!drag || event.pointerId !== drag.id) return;
    const current = drag;
    drag = undefined;
    const dy = event.clientY - current.y;
    if (!current.engaged) {
      if (event.type === 'pointerup' && dy < -24 && Math.abs(dy) > Math.abs(event.clientX - current.x)) {
        suppressClickUntil = Date.now() + 400;
        setChromeCollapsed(true);
      }
      return;
    }
    suppressClickUntil = Date.now() + 400;
    current.samples.push({ t: event.timeStamp, y: event.clientY });
    const height = screenEl?.clientHeight || window.innerHeight;
    const decision = knobRelease({
      offset: dragY(),
      height,
      velocity: releaseVelocity(current.samples),
      cancelled: event.type === 'pointercancel',
    });
    if (decision === 'dismiss') {
      setDragTransition('transform 200ms cubic-bezier(0.2, 0.8, 0.2, 1)');
      setDragY(height);
      clearTimeout(dismissTimer);
      dismissTimer = setTimeout(goHome, 190);
    } else {
      setDragTransition('transform 220ms cubic-bezier(0.2, 0.8, 0.2, 1)');
      setDragY(0);
    }
  }

  /** Swallows the click that ends a chrome drag (it would toggle the
   * knob or hit the exit/picker buttons). */
  function chromeClickCapture(event: MouseEvent): void {
    if (Date.now() < suppressClickUntil) {
      event.stopPropagation();
      event.preventDefault();
    }
  }

  /** Moshi-style title: `host: session`. */
  const hostName = () =>
    hosts.hosts().find((host) => host.id === target?.hostId)?.name ?? target?.sublabel ?? 'host';
  const chromeTitle = () => `${hostName()}: ${target?.label ?? 'shell'}`;
  const routeLabel = () => (path() === 'unknown' ? '…' : pathLabel());
  /** Collapsed-strip dots: route colour, then the multiplexer. */
  const pathDotTone = () =>
    path() === 'relay' ? 'bg-amber' : path() === 'unknown' ? 'bg-text-dim' : 'bg-wm-green';
  const muxDotTone = () =>
    target?.target?.mux === 'herdr' ? 'bg-wm-green' : target?.target?.mux === 'tmux' ? 'bg-wm-cyan' : 'bg-text-dim';

  return (
    <div
      ref={screenEl}
      class="term-screen flex flex-col overflow-hidden bg-bg"
      data-dragging={dragY() > 0 ? '' : undefined}
      style={{
        height: '100%',
        transform: dragY() > 0 ? `translate3d(0, ${dragY()}px, 0)` : undefined,
        transition: dragTransition(),
      }}
    >
      <header
        ref={(el) => {
          chromeEl = el;
          el.addEventListener('click', chromeClickCapture, true);
        }}
        class="term-chrome"
        data-collapsed={collapsed() ? '' : undefined}
        onPointerDown={chromeDown}
        onPointerMove={chromeMove}
        onPointerUp={chromeUp}
        onPointerCancel={chromeUp}
      >
        <button
          type="button"
          aria-label={collapsed() ? 'Expand header' : 'Collapse header'}
          aria-expanded={!collapsed()}
          class="term-chrome-knob"
          onClick={() => setChromeCollapsed(!collapsed())}
        >
          <Show when={collapsed()}>
            <span class={`term-chrome-dot ${pathDotTone()}`} />
          </Show>
          <span class="term-chrome-bar" />
          <Show when={collapsed()}>
            <span class={`term-chrome-dot ${muxDotTone()}`} />
          </Show>
        </button>
        <Show when={!collapsed()}>
          <div class="term-chrome-row">
            <button type="button" aria-label="Exit to Home" class="term-chrome-exit" onClick={goHome}>
              <Minus size={14} strokeWidth={3} />
            </button>
            <button
              type="button"
              aria-label="Switch session"
              class="term-chrome-picker"
              onClick={() => setPickerOpen(true)}
            >
              <List size={13} strokeWidth={2.75} />
            </button>
            <p class="term-chrome-title" title={target?.sublabel}>
              {chromeTitle()}
            </p>
            <span class="term-chrome-route" data-path={path()} aria-label={`Route: ${routeLabel()}`}>
              {routeLabel()}
            </span>
          </div>
        </Show>
      </header>
      <Show when={reconnecting()}>
        <div role="status" class="bg-surface-2 px-4 py-2 text-sm text-text-muted">
          {banner()} · attempt {attempt()}
          <Show when={countdown() !== undefined}> · <span class="tabular-nums">next try in {countdown()}s</span></Show>
        </div>
      </Show>
      <Show when={ended()?.reason === 'exited'}>
        <div class="mx-4 my-3 rounded-card bg-surface-2 p-4">
          <p class="mb-3">Session ended</p>
          <div class="flex gap-3">
            <button
              class="rounded-pill bg-shell px-5 text-face"
              onClick={() => startSession()}
            >
              Reopen
            </button>
            <button class="text-text-muted" onClick={goHome}>
              Back
            </button>
          </div>
        </div>
      </Show>
      <Show when={ended()?.reason === 'error' || ended()?.reason === 'unreachable'}>
        <div class="mx-4 my-3">
          <ErrorBanner
            error={(ended() as { error: AppError }).error}
            retry={() => startSession()}
          />
        </div>
      </Show>
      <Show when={selectionMode()}>
        <div class="flex items-center justify-between bg-surface-2 px-4 py-2 text-sm text-text-muted">
          <span>Selection mode — drag to select</span>
          <span class="flex items-center gap-3">
            <button
              class="rounded-pill bg-shell px-4 py-1 text-[13px] text-face"
              onClick={() => {
                const selected = term?.getSelection();
                if (selected) void clipboardWrite(selected);
                term?.clearSelection();
                setSelectionMode(false);
              }}
            >
              Copy
            </button>
            <button
              class="text-[13px]"
              onClick={() => {
                term?.clearSelection();
                setSelectionMode(false);
              }}
            >
              Done
            </button>
          </span>
        </div>
      </Show>
      <div
        ref={container}
        class="term-touch min-h-0 flex-1"
        onPointerDown={(event) => gestureDown(event)}
        onPointerMove={gestureMove}
        onPointerUp={gestureUp}
        onPointerCancel={gestureUp}
        onContextMenu={(event) => {
          // Desktop (UNI-311): herdr/tmux already get the right-click as a
          // mouse report (xterm's own mousedown handling, unrelated to this
          // DOM event) or draw their own menu from it; only the webview's
          // native context menu is unwanted here, so only that is
          // suppressed. Copy/paste stay reachable via Ctrl+Shift+C/V
          // (lib/desktop.ts termKey, Ctrl+/ shows the full list).
          if (desktopShell()) event.preventDefault();
        }}
      >
        <div
          ref={termHolder}
          class="h-full w-full"
          style={{ 'padding-bottom': 'max(env(safe-area-inset-bottom), var(--inset-bottom, 0px))' }}
        />
        <textarea
          id={props.visible ? 'term-ime' : undefined}
          ref={(el) => {
            imeElRef = el;
          }}
          class="pointer-events-none absolute h-px w-px opacity-0"
          autocapitalize="off"
          autocorrect="off"
          autocomplete="off"
          spellcheck={false}
          enterkeyhint="send"
          tabindex={-1}
        />
      </div>
      <Show when={!desktopShell() && !composerOpen() && !shortcutsOpen() && !historyOpen() && !pickerOpen()}>
        <TermToolbar
          keys={settings.input.toolbar}
          haptics={settings.input.haptics}
          modifierIcons={settings.input.modifierIcons}
          keyboardOpen={keyboardOpen()}
          modifiers={modifiers}
          onBytes={(bytes) => session?.input(bytes)}
          onPaste={() => void pasteClipboard()}
          onShortcuts={() => setShortcutsOpen(true)}
          onHistory={() => setHistoryOpen(true)}
          onComposer={() => setComposerOpen(true)}
          onKeyboardToggle={toggleKeyboard}
        />
      </Show>
      <Show when={composerOpen()}>
        <Composer
          hostId={target?.hostId ?? ''}
          label={target?.label ?? 'shell'}
          onClose={() => setComposerOpen(false)}
          onSend={(bytes) => session?.input(bytes)}
          onShortcuts={() => {
            setComposerOpen(false);
            setShortcutsOpen(true);
          }}
        />
      </Show>
      <Show when={historyOpen()}>
        <HistorySheet
          hostId={target?.hostId ?? ''}
          onClose={() => setHistoryOpen(false)}
          onPick={(text) => {
            setHistoryOpen(false);
            session?.input(composeSend(text));
          }}
        />
      </Show>
      <ShortcutsSheet
        open={shortcutsOpen()}
        onOpenChange={setShortcutsOpen}
        hostId={target?.hostId ?? ''}
      />
      <Show when={target}>
        {(current) => (
          <SessionPicker
            target={{
              hostId: current().hostId,
              hostName: hostName(),
              relay: settings.connection.relay,
            }}
            open={pickerOpen()}
            followFocus={settings.terminal.herdrFollowFocus}
            onOpenChange={setPickerOpen}
            replaceNav
            onSkip={() => {
              setPickerOpen(false);
              setTermTarget({ hostId: current().hostId, target: null, label: 'Shell', sublabel: hostName() });
              navigate('/term', { replace: true });
            }}
          />
        )}
      </Show>
    </div>
  );
}
