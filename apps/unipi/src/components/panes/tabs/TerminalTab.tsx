/**
 * The Terminal tab (WP-A31, P0-05; Z-105..Z-107): xterm.js over a host PTY
 * (`HostClient.openTerm` → `term_open` / `term_attach`, `term_input`,
 * `term_resize`, `term_kill`; host.md §9.3).
 *
 * - One **runtime** per tab id (xterm + its stream) lives in this module, not
 *   in the view: hiding the panel, switching projects or going from the
 *   desktop shell to the phone layer only moves the xterm element, so the
 *   buffer, the TUI on screen and the stream all stay. A reload re-attaches
 *   the tab's terminal (`lib/term` bindings) after restoring the buffer
 *   snapshot taken on unload (the host keeps no scrollback), then nudges the
 *   size so full-screen programs repaint.
 * - Opens in the session's cwd (the pane scope's project); tab names are the
 *   cwd leaf with the lowest free number (Z-105); the PTY exiting closes its
 *   tab; ✕ kills the shell, asking first while a program runs
 *   (`terminalClose.tsx`); the visible xterm takes focus when the user opens
 *   or switches to it.
 * - Desktop: xterm's own keyboard + mouse; Ctrl+Shift+C / Ctrl+Shift+V (⌘C/⌘V)
 *   copy and paste, Ctrl+= / Ctrl+- / Ctrl+0 font size, Mod+J hides the panel;
 *   fit on resize (throttled while dragging); http(s) links open externally.
 * - Phone (`phone` prop, the full-screen layer): the hidden IME textarea is
 *   the only text input (`lib/ime` diffing), `TermToolbar` + `ArrowPad` for
 *   the keys a soft keyboard lacks, a one-line composer, shortcuts and
 *   history drawers, tap to type, drag to scroll.
 */
import { createEffect, createSignal, For, on, onCleanup, onMount, Show, untrack, type Accessor } from 'solid-js';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { SerializeAddon } from '@xterm/addon-serialize';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { WebglAddon } from '@xterm/addon-webgl';
import '@xterm/xterm/css/xterm.css';
import { RotateCw, Send, X } from 'lucide-solid';
import type { AppError } from '@unipi/protocol';
import { appClient, hostClient, type TermStream } from '../../../lib/host';
import { panes, type PaneArea, type PaneScope } from '../../../lib/panes';
import type { PaneTabProps } from '../../../lib/panes/kinds';
import { layout } from '../../../lib/layout';
import {
  bindTabTerm,
  clampDim,
  clearTermSnapshot,
  nextTerminalTitle,
  normalizeTermCwd,
  pushTermHistory,
  readTermHistory,
  readTermSnapshot,
  saveTermSnapshot,
  tabTermBinding,
  TermProcessTracker,
  unbindTabTerm,
} from '../../../lib/term';
import { appTermTheme } from '../../../lib/termTheme';
import { TERM_FONT_FAMILY, onLateFontLoad, termFontsReady } from '../../../lib/termFonts';
import { computeFit } from '../../../lib/termFit';
import { imeKeyBytes, KEYDOWN_TO_KEY, Modifiers, composeSend } from '../../../lib/termToolbar';
import { diffImeValue, imeInitial } from '../../../lib/ime';
import { cellAt, clickSequence, scrollStepPx, scrollTargetFor, wheelSequence, arrowSequence, ScrollAccumulator, SCROLL_SLOP_PX } from '../../../lib/termScroll';
import { termKey, nextFontSize } from '../../../lib/desktop';
import { copyText } from '../../../lib/board/share';
import { openUrlInBrowserTab } from '../../browser/openInBrowserTab';
import { isTauri, platform } from '../../../lib/platform';
import { useSettings } from '../../../settings/context';
import { DEFAULT_SETTINGS, type SettingsV1 } from '../../../settings/schema';
import TermToolbar from '../../TermToolbar';
import { registerTermBusy } from './terminalClose';
import './TerminalTab.css';

// ── Runtime (one per tab id, outlives its view) ───────────────────────────

type Status =
  | { kind: 'connecting' }
  | { kind: 'live' }
  | { kind: 'lost'; message: string }
  | { kind: 'exited' };

type Where = { scope: PaneScope; area: PaneArea; sessionId?: string };

type Runtime = {
  tabId: string;
  hostId: string;
  cwd: string;
  where: Where;
  term: Terminal;
  fit: FitAddon;
  serialize: SerializeAddon;
  /** The element xterm is opened into; views move it in and out. */
  el: HTMLDivElement;
  opened: boolean;
  stream?: TermStream;
  termId?: string;
  shell?: string;
  sent?: { cols: number; rows: number };
  tracker: TermProcessTracker;
  status: Accessor<Status>;
  setStatus: (s: Status) => void;
  /** The mounted view's close (goes through the pane's close flow). */
  closeView?: () => void;
  disposed: boolean;
  cursorTimer?: ReturnType<typeof setTimeout>;
  offBusy: () => void;
  /** xterm's own textarea gets keys only on the desktop shell. */
  phone: boolean;
  /** Quiet re-attaches after a lost writer (bounded). */
  healed: number;
  /** The view moved while following the output: scroll to the end once it fits again. */
  stickBottom: boolean;
};

const runtimes = new Map<string, Runtime>();

type TermOptions = Pick<SettingsV1['terminal'], 'fontSize' | 'lineHeight' | 'cursorStyle' | 'cursorBlink' | 'scrollback'>;

function createRuntime(tabId: string, hostId: string, cwd: string, where: Where, opts: TermOptions): Runtime {
  const term = new Terminal({
    allowProposedApi: true,
    theme: appTermTheme(),
    fontFamily: TERM_FONT_FAMILY,
    fontSize: opts.fontSize,
    lineHeight: opts.lineHeight,
    cursorStyle: opts.cursorStyle,
    cursorBlink: opts.cursorBlink,
    scrollback: opts.scrollback,
    macOptionIsMeta: true,
    rightClickSelectsWord: true,
    drawBoldTextInBrightColors: false,
    minimumContrastRatio: 1,
  });
  const fit = new FitAddon();
  const serialize = new SerializeAddon();
  term.loadAddon(fit);
  term.loadAddon(serialize);
  const unicode = new Unicode11Addon();
  term.loadAddon(unicode);
  term.unicode.activeVersion = '11';
  // Z-106: http(s) links open in the side Browser tab (the system browser where there is none).
  term.loadAddon(new WebLinksAddon((_event, uri) => void openUrlInBrowserTab(uri, { hostId })));
  const el = document.createElement('div');
  el.className = 'term-xterm';
  const [status, setStatus] = createSignal<Status>({ kind: 'connecting' });
  const tracker = new TermProcessTracker();
  const rt: Runtime = { tabId, hostId, cwd, where, term, fit, serialize, el, opened: false, tracker, status, setStatus, disposed: false, offBusy: () => {}, phone: false, healed: 0, stickBottom: false };
  rt.offBusy = registerTermBusy(
    tabId,
    () => ({ running: tracker.isRunning(), hint: tracker.hint(), shell: rt.shell, termId: rt.termId, hostId: rt.hostId }),
    () => disposeRuntime(rt, true),
  );

  term.onData((data) => send(rt, data));
  term.onBinary((data) => send(rt, data));
  term.buffer.onBufferChange((buffer) => tracker.onBufferType(buffer.type));
  term.parser.registerOscHandler(133, (payload) => {
    tracker.onOsc133(payload);
    return false;
  });
  term.attachCustomKeyEventHandler((event) => desktopKey(rt, event));
  runtimes.set(tabId, rt);
  return rt;
}

/** Bytes to the PTY (keys, paste, toolbar, xterm's own replies). */
function send(rt: Runtime, data: string): void {
  if (!data || rt.disposed) return;
  rt.tracker.onInput(data);
  rt.stream?.write(data);
}

/** The cursor row once output settles: a prompt there ends a tracked command. */
function scheduleCursorCheck(rt: Runtime): void {
  if (!rt.tracker.isRunning() || rt.cursorTimer) return;
  rt.cursorTimer = setTimeout(() => {
    rt.cursorTimer = undefined;
    const buffer = rt.term.buffer.active;
    if (buffer.type !== 'normal') return;
    const line = buffer.getLine(buffer.baseY + buffer.cursorY);
    rt.tracker.onCursorLine(line?.translateToString(true, 0, buffer.cursorX) ?? '');
  }, 120);
}

function connect(rt: Runtime, attachId?: string): void {
  rt.stream?.close();
  rt.sent = undefined;
  rt.setStatus({ kind: 'connecting' });
  const cols = rt.term.cols;
  const rows = rt.term.rows;
  // Re-attach one row short, then resize to fit: the size change makes
  // full-screen programs (vi, top) repaint what the reload lost.
  const nudge = !!attachId && rows > 2;
  let opened = false;
  const stream = hostClient(rt.hostId).openTerm(
    { cwd: normalizeTermCwd(rt.cwd), cols, rows: nudge ? rows - 1 : rows, id: attachId },
    {
      data(bytes) {
        if (rt.stream !== stream) return;
        rt.term.write(bytes);
        scheduleCursorCheck(rt);
      },
      opened(info) {
        if (rt.stream !== stream || rt.disposed) return;
        opened = true;
        rt.termId = info.termId;
        rt.shell = info.shell;
        bindTabTerm(rt.tabId, { termId: info.termId, hostId: rt.hostId, cwd: rt.cwd, shell: info.shell });
        rt.setStatus({ kind: 'live' });
        rt.sent = { cols, rows: nudge ? rows - 1 : rows };
        syncSize(rt);
      },
      title() {},
      exit() {
        if (rt.stream !== stream) return;
        rt.tracker.onExit();
        rt.setStatus({ kind: 'exited' });
        // Z-105: the PTY exiting closes its tab (and the panel with the last one).
        closeTab(rt);
      },
      closed(error?: AppError) {
        if (rt.stream !== stream || rt.disposed) return;
        if (attachId && !opened && error?.code === 'E_NOT_FOUND') {
          // The terminal is gone (host restarted, shell ended while away): start a fresh one here.
          unbindTabTerm(rt.tabId);
          clearTermSnapshot(attachId);
          rt.termId = undefined;
          const b = rt.term.buffer.active;
          const blank = b.baseY === 0 && b.cursorY === 0 && b.cursorX === 0;
          rt.term.write(`${blank ? '' : '\r\n'}\x1b[2m[The previous shell has ended — new shell]\x1b[0m\r\n`);
          connect(rt);
          return;
        }
        // The app dropped the stream's writer (a stale pre-reload stream ending): re-attach quietly.
        if (opened && error?.code === 'E_NOT_FOUND' && rt.termId && rt.healed < 3) {
          rt.healed += 1;
          connect(rt, rt.termId);
          return;
        }
        rt.setStatus({ kind: 'lost', message: error?.message ? `${error.message}${error.code ? ` (${error.code})` : ''}` : 'The connection to the terminal was lost.' });
      },
    },
  );
  rt.stream = stream;
}

/** Sends the xterm size to the PTY when it changed. */
function syncSize(rt: Runtime): void {
  const { cols, rows } = rt.term;
  if (!rt.stream || rt.status().kind !== 'live') return;
  if (rt.sent && rt.sent.cols === cols && rt.sent.rows === rows) return;
  rt.sent = { cols, rows };
  rt.stream.resize(clampDim(cols, 80), clampDim(rows, 24));
}

function closeTab(rt: Runtime): void {
  if (rt.disposed) return;
  if (rt.closeView) {
    rt.closeView();
    return;
  }
  const store = panes();
  const view = { sessionId: rt.where.sessionId };
  store.close(rt.where.scope, rt.where.area, view, rt.tabId);
  if (rt.where.area === 'bottom' && store.visible(rt.where.scope, 'bottom', view).length === 0) layout().toggleBottom(false);
  disposeRuntime(rt, true);
}

function tabExists(where: Where, tabId: string): boolean {
  return panes()
    .set(where.scope, where.area)
    .tabs.some((t) => t.id === tabId);
}

/** Ends a runtime: `kill` ends the host shell too (the tab was closed). */
function disposeRuntime(rt: Runtime, kill: boolean): void {
  if (rt.disposed) return;
  rt.disposed = true;
  runtimes.delete(rt.tabId);
  rt.offBusy();
  clearTimeout(rt.cursorTimer);
  const stream = rt.stream;
  rt.stream = undefined;
  if (kill) {
    if (stream?.kill) void stream.kill();
    else stream?.close();
    unbindTabTerm(rt.tabId);
    if (rt.termId) clearTermSnapshot(rt.termId);
  } else stream?.close();
  rt.term.dispose();
  rt.el.remove();
}

/** Desktop keys (`lib/desktop` `termKey`): false = handled here, xterm skips it. */
function desktopKey(rt: Runtime, event: KeyboardEvent): boolean {
  if (rt.phone) return true;
  const mac = /Mac/.test(navigator.platform || '');
  // Esc belongs to the program (vi, less), not the app's Back.
  if (event.key === 'Escape') event.stopPropagation();
  // Mod+J hides the bottom panel from inside the terminal too (plain Ctrl+J is a newline otherwise).
  if (event.type === 'keydown' && rt.where.area === 'bottom' && (mac ? event.metaKey : event.ctrlKey) && !event.shiftKey && !event.altKey && event.key.toLowerCase() === 'j') {
    event.preventDefault();
    layout().toggleBottom(false);
    return false;
  }
  const action = termKey(event, mac);
  if (!action) return true;
  if (event.type !== 'keydown') return false;
  if (action.kind === 'app') return false;
  event.preventDefault();
  if (action.kind === 'copy') {
    const selected = rt.term.getSelection();
    if (selected) void copyText(selected);
  } else if (action.kind === 'paste') void pasteInto(rt);
  else if (action.kind === 'font') fontStep?.(action.delta);
  return false;
}

/** Set by the mounted view (it owns the settings store). */
let fontStep: ((delta: 1 | -1 | 0) => void) | undefined;

async function readClipboard(): Promise<string | undefined> {
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

async function pasteInto(rt: Runtime): Promise<void> {
  const text = await readClipboard();
  // xterm.paste: CRLF → CR and bracketed paste when the program turned it on.
  if (text && !rt.disposed) rt.term.paste(text);
}

// Reload: snapshot every live buffer so the re-attached tabs start where they were.
if (typeof window !== 'undefined') {
  const snapshotAll = () => {
    for (const rt of runtimes.values()) {
      if (!rt.termId || !rt.opened || rt.disposed) continue;
      try {
        saveTermSnapshot(rt.termId, rt.serialize.serialize({ scrollback: 1000 }));
      } catch {
        // a snapshot is best effort
      }
    }
  };
  window.addEventListener('beforeunload', snapshotAll);
  window.addEventListener('pagehide', snapshotAll);
}

/** Has the user just acted (click / key)? Restores after a reload don't steal focus. */
function userActive(): boolean {
  const ua = (navigator as Navigator & { userActivation?: { isActive: boolean } }).userActivation;
  return ua ? ua.isActive : true;
}

function cellSize(term: Terminal): { width: number; height: number } | undefined {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const cell = (term as any)._core?._renderService?.dimensions?.css?.cell as { width: number; height: number } | undefined;
  return cell && cell.width > 0 && cell.height > 0 ? cell : undefined;
}

// ── Settings (the phone layer and tests may sit outside the provider) ──────

function useTermSettings(): [SettingsV1, (path: string, value: unknown) => unknown] {
  try {
    return useSettings();
  } catch {
    return [DEFAULT_SETTINGS, () => undefined];
  }
}

// ── The view ──────────────────────────────────────────────────────────────

export type TerminalViewProps = {
  tabId: string;
  hostId: string;
  cwd: string;
  where: Where;
  /** The phone layer: IME + toolbar input instead of xterm's own keyboard. */
  phone?: boolean;
  /** Closes the tab through its pane (the ✕ flow). */
  close: () => void;
};

export function TerminalView(props: TerminalViewProps) {
  const [settings, update] = useTermSettings();
  const phone = () => !!props.phone;
  let holder!: HTMLDivElement;
  let ime: HTMLTextAreaElement | undefined;
  const tabId = props.tabId;
  const where = props.where;
  const rt =
    runtimes.get(tabId) ??
    createRuntime(tabId, props.hostId, props.cwd, where, untrack(() => ({ ...settings.terminal })));
  rt.phone = phone();
  rt.closeView = () => props.close();
  const [drawer, setDrawer] = createSignal<'composer' | 'shortcuts' | 'history' | undefined>();
  const [keyboardOpen, setKeyboardOpen] = createSignal(false);
  const modifiers = new Modifiers();
  let imeState = imeInitial();
  let disposed = false;

  fontStep = (delta) => {
    const next = nextFontSize(settings.terminal.fontSize, delta);
    if (next !== settings.terminal.fontSize) update('terminal.fontSize', next);
  };

  // Fit: throttled while the panel is dragged; trailing call lands the final size.
  let fitTimer: ReturnType<typeof setTimeout> | undefined;
  let lastFit = 0;
  let wasVisible = false;
  const fitNow = () => {
    fitTimer = undefined;
    lastFit = performance.now();
    if (!rt.opened || rt.disposed || disposed) return;
    const box = holder.getBoundingClientRect();
    if (box.width < 20 || box.height < 20) return;
    let dims: { cols: number; rows: number } | undefined;
    if (phone()) {
      const cell = cellSize(rt.term);
      const pad = 12;
      dims = cell ? computeFit({ containerWidth: box.width - pad, containerHeight: box.height - 8, cellWidth: cell.width, cellHeight: cell.height, basePaddingX: 0, basePaddingY: 0 }) : undefined;
    } else dims = rt.fit.proposeDimensions();
    if (!dims || !Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return;
    const cols = Math.max(2, dims.cols);
    const rows = Math.max(1, dims.rows);
    if (cols !== rt.term.cols || rows !== rt.term.rows) {
      // Keep following the output across a reflow (a resize can leave the viewport mid-buffer).
      const buf = rt.term.buffer.active;
      const follow = rt.stickBottom || buf.viewportY >= buf.baseY;
      rt.term.resize(cols, rows);
      if (follow) rt.term.scrollToBottom();
    } else if (rt.stickBottom) rt.term.scrollToBottom();
    rt.stickBottom = false;
    syncSize(rt);
  };
  const scheduleFit = () => {
    if (fitTimer) return;
    const wait = Math.max(0, 60 - (performance.now() - lastFit));
    fitTimer = setTimeout(() => requestAnimationFrame(fitNow), wait);
  };

  const focusTerm = () => {
    if (phone()) return;
    rt.term.focus();
  };

  onMount(() => {
    holder.appendChild(rt.el);
    const observer = new ResizeObserver(() => {
      const box = holder.getBoundingClientRect();
      const visible = box.width > 20 && box.height > 20;
      if (visible && !wasVisible && rt.opened && userActive()) focusTerm();
      wasVisible = visible;
      scheduleFit();
    });
    observer.observe(holder);
    const stopLateFonts = onLateFontLoad(() => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      (rt.term as any)._core?._charSizeService?.measure?.();
      if (rt.opened) rt.term.refresh(0, rt.term.rows - 1);
      scheduleFit();
    });
    void (async () => {
      if (!rt.opened) {
        // xterm measures its cell when it opens: fonts first, and a laid-out holder.
        await termFontsReady(settings.terminal.fontSize);
        for (let i = 0; i < 60 && holder.getBoundingClientRect().width < 20 && !disposed; i++) await new Promise((r) => requestAnimationFrame(r));
        if (disposed || rt.disposed || rt.opened) return;
        rt.term.open(rt.el);
        rt.opened = true;
        const textarea = rt.term.textarea;
        if (textarea) {
          for (const [k, v] of [['autocomplete', 'off'], ['autocorrect', 'off'], ['autocapitalize', 'off'], ['spellcheck', 'false']]) textarea.setAttribute(k, v);
        }
        try {
          // WebGL is the fast path, but WebKitGTK (the Linux desktop webview) leaves stale
          // frames with it (a full-screen program's first paint never shows): DOM renderer there.
          if (!('__UNIPI_NO_WEBGL__' in window) && platform().os !== 'linux') {
            const webgl = new WebglAddon();
            webgl.onContextLoss(() => webgl.dispose());
            rt.term.loadAddon(webgl);
          }
        } catch {
          // DOM renderer
        }
        fitNow();
        const binding = tabTermBinding(tabId);
        if (binding && binding.hostId === rt.hostId) {
          const snap = readTermSnapshot(binding.termId);
          if (snap) rt.term.write(snap);
          rt.termId = binding.termId;
          rt.shell = binding.shell;
          // After a reload the app may still hold this terminal's pre-reload stream under the same
          // id: detach it first, so its late end can't take the new stream's writer with it.
          await appClient().request('term_close', { termId: binding.termId }).catch(() => {});
          await new Promise((r) => setTimeout(r, 250));
          if (rt.disposed) return;
          connect(rt, binding.termId);
        } else connect(rt);
        wasVisible = true;
        if (userActive()) focusTerm();
      } else {
        fitNow();
        rt.term.refresh(0, rt.term.rows - 1);
        wasVisible = true;
        if (userActive()) focusTerm();
      }
    })();

    onCleanup(() => {
      disposed = true;
      observer.disconnect();
      stopLateFonts();
      clearTimeout(fitTimer);
      if (rt.closeView) rt.closeView = undefined;
      // Still a tab (panel hidden, other project, phone ⇄ desktop): keep it running.
      if (rt.disposed) return;
      if (tabExists(where, tabId)) {
        const buf = rt.term.buffer.active;
        rt.stickBottom = buf.viewportY >= buf.baseY;
        rt.el.remove();
      }
      else disposeRuntime(rt, true);
    });
  });

  // Settings follow live.
  createEffect(
    on(
      () => [settings.terminal.fontSize, settings.terminal.lineHeight, settings.terminal.cursorStyle, settings.terminal.cursorBlink, settings.terminal.scrollback] as const,
      ([fontSize, lineHeight, cursorStyle, cursorBlink, scrollback]) => {
        const o = rt.term.options;
        if (o.fontSize !== fontSize) o.fontSize = fontSize;
        if (o.lineHeight !== lineHeight) o.lineHeight = lineHeight;
        if (o.cursorStyle !== cursorStyle) o.cursorStyle = cursorStyle;
        if (o.cursorBlink !== cursorBlink) o.cursorBlink = cursorBlink;
        if (o.scrollback !== scrollback) o.scrollback = scrollback;
        scheduleFit();
      },
      { defer: true },
    ),
  );

  // ── Phone input: the IME textarea, toolbar, gestures ────────────────────
  const sendPhone = (bytes: string) => {
    send(rt, bytes);
    if (rt.term.buffer.active.viewportY !== rt.term.buffer.active.baseY) rt.term.scrollToBottom();
  };
  const onImeInput = () => {
    if (!ime) return;
    const result = diffImeValue(ime.value, imeState);
    imeState = result.state;
    if (result.bytes) {
      const modified = modifiers.applyToText(result.bytes);
      sendPhone(modified.bytes);
      if (modified.applied) {
        ime.value = '';
        imeState = imeInitial();
        return;
      }
    }
    if (result.resetValue !== undefined) ime.value = result.resetValue;
  };
  const onImeKey = (event: KeyboardEvent) => {
    if (event.key.length === 1 && !event.ctrlKey && !event.altKey && !event.metaKey) return;
    const named = KEYDOWN_TO_KEY[event.key];
    if (named && modifiers.anyActive() && !event.ctrlKey && !event.altKey) {
      event.preventDefault();
      sendPhone(modifiers.consume(named));
      return;
    }
    const bytes = imeKeyBytes(event.key, event.ctrlKey, event.altKey);
    if (!bytes) return;
    event.preventDefault();
    if (event.key === 'Enter' && ime) {
      ime.value = '';
      imeState = imeInitial();
    }
    sendPhone(bytes);
  };
  const toggleKeyboard = () => {
    if (!ime) return;
    if (keyboardOpen()) {
      ime.blur();
      return;
    }
    if (document.activeElement === ime) ime.blur();
    ime.focus();
  };

  // Touch: tap = type here (and click for mouse-aware programs), vertical drag = scroll.
  let down: { x: number; y: number; id: number } | undefined;
  let scrolling = false;
  let lastY = 0;
  let acc: ScrollAccumulator | undefined;
  const scrollPlan = () => {
    const cell = cellSize(rt.term);
    if (!cell) return undefined;
    const kind = scrollTargetFor(rt.term.modes.mouseTrackingMode, rt.term.buffer.active.type);
    return { cell, kind, stepPx: scrollStepPx(kind, cell.height) };
  };
  const emitScroll = (steps: number, at: { x: number; y: number }) => {
    const plan = scrollPlan();
    if (!plan || steps === 0) return;
    const dir = steps > 0 ? 'up' : 'down';
    const n = Math.abs(steps);
    if (plan.kind === 'scrollback') {
      rt.term.scrollLines(steps > 0 ? -n : n);
      return;
    }
    const screen = rt.el.querySelector('.xterm-screen')?.getBoundingClientRect();
    const cell = cellAt(at, { left: screen?.left ?? 0, top: screen?.top ?? 0 }, plan.cell, { cols: rt.term.cols, rows: rt.term.rows });
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const encoding = (rt.term as any)._core?.coreMouseService?.activeEncoding ?? 'SGR';
    const seq = plan.kind === 'mouse' ? wheelSequence(dir, cell.col, cell.row, encoding) : arrowSequence(dir, rt.term.modes.applicationCursorKeysMode);
    send(rt, seq.repeat(n));
  };
  let lastTouch = 0;
  const onPointerDown = (e: PointerEvent) => {
    if (!phone() || e.pointerType === 'mouse') return;
    lastTouch = performance.now();
    down = { x: e.clientX, y: e.clientY, id: e.pointerId };
    scrolling = false;
    lastY = e.clientY;
    const plan = scrollPlan();
    acc = plan ? new ScrollAccumulator(plan.stepPx) : undefined;
  };
  const onPointerMove = (e: PointerEvent) => {
    if (!down || e.pointerId !== down.id) return;
    if (!scrolling && Math.abs(e.clientY - down.y) > SCROLL_SLOP_PX) scrolling = true;
    if (!scrolling || !acc) return;
    const steps = acc.push(e.clientY - lastY);
    lastY = e.clientY;
    emitScroll(steps, { x: e.clientX, y: e.clientY });
  };
  const onPointerUp = (e: PointerEvent) => {
    if (!down || e.pointerId !== down.id) return;
    const tap = !scrolling;
    down = undefined;
    if (!tap) return;
    if (rt.term.modes.mouseTrackingMode !== 'none') {
      const cell = cellSize(rt.term);
      const screen = rt.el.querySelector('.xterm-screen')?.getBoundingClientRect();
      if (cell && screen) {
        const at = cellAt({ x: e.clientX, y: e.clientY }, { left: screen.left, top: screen.top }, cell, { cols: rt.term.cols, rows: rt.term.rows });
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const encoding = (rt.term as any)._core?.coreMouseService?.activeEncoding ?? 'SGR';
        send(rt, clickSequence(at.col, at.row, encoding));
      }
    }
    ime?.focus();
  };
  // On the phone a touch must not reach xterm as a mouse press (focus, selection, X10 reports).
  const blockCompat = (e: MouseEvent) => {
    if (!phone() || performance.now() - lastTouch > 800) return;
    e.preventDefault();
    e.stopPropagation();
  };

  onMount(() => {
    if (!phone()) return;
    const onViewport = () => {
      const keyboard = Math.max(0, window.innerHeight - (window.visualViewport?.height ?? window.innerHeight), (window as unknown as { unipiImeInset?: number }).unipiImeInset ?? 0);
      setKeyboardOpen(keyboard > 120 || document.activeElement === ime);
      scheduleFit();
    };
    window.visualViewport?.addEventListener('resize', onViewport);
    window.addEventListener('unipi-ime', onViewport);
    // xterm focuses its helper textarea on presses; on the phone the IME textarea is the input.
    const bounce = (e: FocusEvent) => {
      if (e.target && e.target === rt.term.textarea) ime?.focus();
    };
    document.addEventListener('focusin', bounce);
    holder.addEventListener('mousedown', blockCompat, true);
    holder.addEventListener('mouseup', blockCompat, true);
    onCleanup(() => {
      holder.removeEventListener('mousedown', blockCompat, true);
      holder.removeEventListener('mouseup', blockCompat, true);
      window.visualViewport?.removeEventListener('resize', onViewport);
      window.removeEventListener('unipi-ime', onViewport);
      document.removeEventListener('focusin', bounce);
    });
  });

  // ── Phone drawers ───────────────────────────────────────────────────────
  const [draft, setDraft] = createSignal('');
  const [history, setHistory] = createSignal<string[]>([]);
  const submitDraft = () => {
    const text = draft();
    if (!text.trim()) return;
    setHistory(pushTermHistory(props.hostId, text));
    sendPhone(composeSend(text));
    setDraft('');
    setDrawer(undefined);
  };
  const openDrawer = (which: 'composer' | 'shortcuts' | 'history') => {
    if (which === 'history') setHistory(readTermHistory(props.hostId));
    setDrawer(drawer() === which ? undefined : which);
  };

  const status = () => rt.status();
  const lost = () => {
    const s = status();
    return s.kind === 'lost' ? s : undefined;
  };

  return (
    <div class="term-pane" data-testid="terminal-tab" data-term-tab={tabId} data-phone={phone() ? '' : undefined} data-status={status().kind}>
      <Show when={lost()}>
        {(s) => (
          <div class="term-status" role="status" data-testid="term-lost">
            <span class="term-status-dot" aria-hidden="true" />
            <span class="term-status-text" title={s().message}>
              Disconnected — {s().message}
            </span>
            <button type="button" class="term-status-btn" data-testid="term-reconnect" onClick={() => connect(rt, rt.termId)}>
              <RotateCw size={13} />
              Reconnect
            </button>
          </div>
        )}
      </Show>
      <div
        class="term-surface"
        ref={holder}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => (down = undefined)}
        onContextMenu={(e) => phone() && e.preventDefault()}
      />
      <Show when={phone()}>
        <textarea
          ref={ime}
          class="term-ime"
          data-testid="term-ime"
          aria-label="Terminal input"
          autocomplete="off"
          autocapitalize="off"
          spellcheck={false}
          enterkeyhint="send"
          onInput={onImeInput}
          onKeyDown={onImeKey}
          onFocus={() => setKeyboardOpen(true)}
          onBlur={() => setKeyboardOpen(false)}
        />
        <Show when={drawer()}>
          {(which) => (
            <div class="term-drawer" data-testid={`term-drawer-${which()}`}>
              <div class="term-drawer-head">
                <p class="term-drawer-title">{which() === 'composer' ? 'Command' : which() === 'shortcuts' ? 'Shortcuts' : 'History'}</p>
                <button type="button" class="term-drawer-close" aria-label="Close" onMouseDown={(e) => e.preventDefault()} onClick={() => setDrawer(undefined)}>
                  <X size={16} />
                </button>
              </div>
              <Show when={which() === 'composer'}>
                <div class="term-composer-row">
                  <textarea
                    class="term-composer-input"
                    rows={1}
                    placeholder="Type a command…"
                    value={draft()}
                    autofocus
                    autocomplete="off"
                    autocapitalize="off"
                    spellcheck={false}
                    enterkeyhint="send"
                    ref={(el) => queueMicrotask(() => el.focus())}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' && !e.shiftKey) {
                        e.preventDefault();
                        submitDraft();
                      }
                    }}
                  />
                  <button type="button" class="term-composer-send" aria-label="Run" disabled={!draft().trim()} onClick={submitDraft}>
                    <Send size={17} />
                  </button>
                </div>
              </Show>
              <Show when={which() === 'shortcuts'}>
                <div class="term-shortcuts-grid">
                  <For each={SHORTCUTS}>
                    {(s) => (
                      <button type="button" class="term-shortcut-chip" onMouseDown={(e) => e.preventDefault()} onClick={() => sendPhone(s.bytes)}>
                        <span>{s.label}</span>
                        <span class="term-shortcut-key">{s.key}</span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
              <Show when={which() === 'history'}>
                <Show when={history().length > 0} fallback={<p class="term-history-empty">Commands you run from the composer show up here.</p>}>
                  <div class="term-history-list">
                    <For each={history()}>
                      {(cmd) => (
                        <button
                          type="button"
                          class="term-history-row"
                          title={cmd}
                          onClick={() => {
                            setDraft(cmd);
                            setDrawer('composer');
                          }}
                        >
                          {cmd}
                        </button>
                      )}
                    </For>
                  </div>
                </Show>
              </Show>
            </div>
          )}
        </Show>
        <TermToolbar
          keys={settings.input.toolbar}
          haptics={settings.input.haptics}
          modifierIcons={settings.input.modifierIcons}
          keyboardOpen={keyboardOpen()}
          modifiers={modifiers}
          onBytes={sendPhone}
          onPaste={() => void pasteInto(rt)}
          onShortcuts={() => openDrawer('shortcuts')}
          onHistory={() => openDrawer('history')}
          onComposer={() => openDrawer('composer')}
          onKeyboardToggle={toggleKeyboard}
        />
      </Show>
    </div>
  );
}

const SHORTCUTS: { label: string; key: string; bytes: string }[] = [
  { label: 'Interrupt', key: '^C', bytes: '\x03' },
  { label: 'End of input', key: '^D', bytes: '\x04' },
  { label: 'Suspend', key: '^Z', bytes: '\x1a' },
  { label: 'Clear screen', key: '^L', bytes: '\x0c' },
  { label: 'Search history', key: '^R', bytes: '\x12' },
  { label: 'Line start', key: '^A', bytes: '\x01' },
  { label: 'Line end', key: '^E', bytes: '\x05' },
  { label: 'Delete word', key: '^W', bytes: '\x17' },
  { label: 'Delete line', key: '^U', bytes: '\x15' },
  { label: 'Quit (vi)', key: ':q', bytes: '\x1b:q\r' },
];

// ── The pane tab (desktop side pane / bottom panel) ───────────────────────

export function TerminalTab(props: PaneTabProps) {
  const cwd = () => String(props.tab.params?.cwd ?? props.projectKey ?? '~');
  const where: Where = { scope: { hostId: props.hostId, projectKey: props.projectKey }, area: props.area, sessionId: props.sessionId };
  // Z-105: name a fresh tab after its folder ("oino", "oino 2", …).
  onMount(() => {
    if (props.tab.title) return;
    const others = panes()
      .set(where.scope, where.area)
      .tabs.filter((t) => t.kind === 'terminal' && t.id !== props.tab.id)
      .map((t) => t.title ?? '');
    props.setTitle(nextTerminalTitle(cwd(), others));
  });
  return <TerminalView tabId={props.tab.id} hostId={props.hostId} cwd={cwd()} where={where} close={props.close} />;
}

/** Is this tab's terminal live in this window (tests, the phone layer)? */
export const terminalRuntimeOf = (tabId: string): { termId?: string; running: boolean } | undefined => {
  const rt = runtimes.get(tabId);
  return rt ? { termId: rt.termId, running: rt.tracker.isRunning() } : undefined;
};

export default TerminalTab;
