/**
 * Preview → Browser (UNI-215): view and drive a dedicated desktop browser on
 * the PC. `BrowserPanel` is the pre-start card (status: off on the PC, no
 * browser installed, ready with its open tabs); `BrowserViewer` is the
 * full-screen live view: our own tab strip, URL bar with back/forward/
 * reload, a canvas showing the PC's JPEG frames, and touch/mouse/keyboard
 * mapped to CDP input. Rotating or resizing re-sizes the PC's viewport.
 *
 * Gestures on the canvas (phone): tap = click, drag = scroll (wheel),
 * long-press = right click, two-finger pinch = page zoom. Mouse (desktop):
 * click/drag/select, wheel scrolls. Keyboard: the ⌨ button raises the soft
 * keyboard (a hidden input diffed like the terminal's IME); on desktop
 * typing goes straight through while the canvas has focus.
 *
 * UNI-296: `BrowserViewer` is also the browser beside the chat — `variant`
 * `pane` (desktop side pane) / `sheet` (phone): a header with the profile
 * picker and the engine/stealth badge, then the URL row. Every variant shows
 * the "Agent is using the browser · Take over" strip while an agent drives
 * (and "You have control · Give back" while taken over).
 */
import { createEffect, createSignal, For, Match, onCleanup, onMount, Show, Switch, type JSX } from 'solid-js';
import {
  ArrowLeft,
  ArrowRight,
  AppWindow,
  Camera,
  Check,
  ChevronDown,
  Hand,
  ShieldAlert,
  ShieldCheck,
  EllipsisVertical,
  EyeOff,
  Globe,
  Keyboard,
  MonitorPlay,
  Plus,
  Power,
  RefreshCw,
  RotateCw,
  Square,
  SquareMousePointer,
  TriangleAlert,
  Unplug,
  X,
} from 'lucide-solid';
import type { BrowserElement, BrowserEvent, BrowserOpenData, BrowserProfilesData, BrowserStatusData, BrowserTab, BrowserViewport } from '@unipi/protocol';
import type { Result } from '../lib/ipc';
import { pushBack } from '../lib/back';
import { imeInitial } from '../lib/ime';
import {
  BrowserSession,
  browserClose,
  browserName,
  browserProfiles,
  browserStatus,
  browserTakeover,
  browserWindow,
  canShowWindow,
  windowShown,
  displayUrl,
  ENABLE_BROWSER_COMMAND,
  imeCommands,
  INSTALL_STEPS,
  keyCommand,
  modifiers,
  sameViewport,
  statusView,
  tabLabel,
  toPage,
  viewportFor,
  type Frame,
  type SessionProblem,
  type SessionState,
} from '../lib/browser';
import { isTauriMobile } from '../lib/platform';
import { agentStrip, canPickProfile, cropRect, followTab, overlayRect, type BrowserPick, type BrowserShot, profileChoices, stealthBadge, takeover, type TakeoverAction, type TakeoverState } from '../lib/browserPane';
import './preview.css';
import { ErrorBanner } from '../components/ErrorBanner';
import { isUnreachable, OfflineState } from '../components/EmptyState';
import { FeatureOff } from '../components/FeatureOff';
import { LeaveButton, LeaveSheet } from '../components/LeaveSheet';
import { AnchoredMenu, MenuLabel, MenuRow, MenuSeparator } from '../components/board/AnchoredMenu';

type CommandBoxComp = (props: { command: string }) => JSX.Element;
type NoticeComp = (props: { icon: JSX.Element; title: string; tone?: 'warn' | 'error'; testId?: string; children: JSX.Element }) => JSX.Element;

/** The pre-start card in the Preview page's Browser mode. */
export function BrowserPanel(props: {
  hostId: string;
  hostName: string;
  Notice: NoticeComp;
  CommandBox: CommandBoxComp;
  onOpen: (args: { url?: string; target?: string }) => void;
}) {
  const [status, setStatus] = createSignal<Result<BrowserStatusData> | undefined>();
  const [refreshing, setRefreshing] = createSignal(false);
  const [url, setUrl] = createSignal('');
  const [closing, setClosing] = createSignal(false);
  const [leaving, setLeaving] = createSignal(false);
  async function refresh() {
    const id = props.hostId;
    setRefreshing(true);
    const result = await browserStatus(id);
    setRefreshing(false);
    if (id === props.hostId) setStatus(result);
  }
  createEffect(() => {
    props.hostId;
    setStatus(undefined);
    void refresh();
  });
  const view = () => statusView(status());
  const { Notice, CommandBox } = props;

  return (
    <div class="browser-panel" data-testid="browser-panel">
      <Show when={view().kind === 'loading'}>
        <div class="preview-loading" data-testid="browser-loading">
          <RefreshCw size={18} class="refresh-spinning" /> Checking the browser on {props.hostName}…
        </div>
      </Show>
      <Show when={view().kind === 'disabled'}>
        <div data-testid="browser-disabled">
          <FeatureOff feature="browser" hostId={props.hostId} hostName={props.hostName} CommandBox={CommandBox} onChanged={() => void refresh()} />
        </div>
      </Show>
      <Show when={view().kind === 'not-found'}>
        <Notice icon={<Globe size={22} />} title="Google Chrome isn't installed on the PC" tone="warn" testId="browser-not-found">
          <InstallChrome CommandBox={CommandBox} />
          <p class="preview-muted">
            Installed somewhere unusual? Point the agent host at it: <code>unipi-oino-host browser enable --path /path/to/chrome</code>
          </p>
          <button type="button" class="preview-btn" onClick={() => void refresh()}>
            <RefreshCw size={16} class={refreshing() ? 'refresh-spinning' : ''} /> Check again
          </button>
        </Notice>
      </Show>
      <Show when={view().kind === 'error' && view()}>
        {(v) => {
          const err = (v() as { error: Parameters<typeof ErrorBanner>[0]['error'] }).error;
          return (
            <Show when={isUnreachable(err.code)} fallback={<ErrorBanner error={err} retry={() => void refresh()} />}>
              <OfflineState compact pc={props.hostName} message={err.message} hint={err.hint} onRetry={() => void refresh()} />
            </Show>
          );
        }}
      </Show>
      <Show when={view().kind === 'ready' && (view() as { status: BrowserStatusData }).status}>
        {(s) => (
          <>
            <form
              class="browser-start"
              onSubmit={(e) => {
                e.preventDefault();
                // Drop the keyboard: the viewer opens over this page and the IME would cover the stream.
                (document.activeElement as HTMLElement | null)?.blur?.();
                props.onOpen({ url: url().trim() || undefined });
              }}
            >
              <label class="section-label" for="browser-url-input">Open a page</label>
              <div class="preview-manual-row">
                <input
                  id="browser-url-input"
                  class="preview-input browser-start-input"
                  placeholder="example.com or localhost:5173"
                  value={url()}
                  autocapitalize="off"
                  autocorrect="off"
                  spellcheck={false}
                  enterkeyhint="go"
                  inputmode="url"
                  onInput={(e) => setUrl(e.currentTarget.value)}
                />
                <button type="submit" class="preview-btn primary" data-testid="browser-start">
                  <MonitorPlay size={17} /> Open
                </button>
              </div>
            </form>
            <div class="browser-info" data-testid="browser-ready">
              <span class="pc-dot" data-state={s().running ? 'ok' : 'idle'} />
              <span class="browser-info-text">
                {browserName(s().browser?.name)} on {props.hostName} ·{' '}
                {s().running ? 'running' : 'starts when you open it'} · {windowShown(s()) ? 'window visible on the PC' : 'only shown here'}
              </span>
            </div>
            <Show when={s().tabs.length > 0}>
              <h2 class="section-label">Open tabs on the PC</h2>
              <div class="preview-list" data-testid="browser-tabs-list">
                <For each={s().tabs}>
                  {(tab) => (
                    <button type="button" class="preview-row preview-row-main" onClick={() => props.onOpen({ target: tab.id })}>
                      <span class="browser-tab-icon"><Globe size={16} /></span>
                      <span class="preview-row-text">
                        <span class="preview-row-name"><span class="truncate">{tabLabel(tab)}</span></span>
                        <span class="preview-row-meta">{displayUrl(tab.url) || 'about:blank'}</span>
                      </span>
                    </button>
                  )}
                </For>
              </div>
              <button
                type="button"
                class="preview-btn browser-quit"
                disabled={closing()}
                onClick={async () => {
                  setClosing(true);
                  await browserClose(props.hostId);
                  setClosing(false);
                  void refresh();
                }}
              >
                <Power size={16} /> Quit the browser on the PC
              </button>
            </Show>
            <p class="preview-muted browser-note">
              A separate browser profile on the PC — your own browser and its logins are never touched. It quits after{' '}
              {s().idle_minutes ? `${s().idle_minutes} min` : 'never'} with nobody watching.
            </p>
            {/* UNI-253: a phone can stop using a feature it has. */}
            <div class="browser-leave">
              <LeaveButton onClick={() => setLeaving(true)} label="Turn off the browser for this phone" testId="browser-leave" />
            </div>
            <LeaveSheet open={leaving()} onClose={() => setLeaving(false)} hostId={props.hostId} feature="browser" allowed />
          </>
        )}
      </Show>
    </div>
  );
}

/** "Install Google Chrome on the PC" with the way per OS. */
function InstallChrome(props: { CommandBox: CommandBoxComp }) {
  return (
    <div class="browser-install" data-testid="browser-install">
      <p>Install Google Chrome on the PC. The remote browser runs its own copy with a separate profile.</p>
      <For each={INSTALL_STEPS}>
        {(step) => (
          <div class="browser-install-step">
            <p class="preview-muted"><strong>{step.os}:</strong> {step.text}</p>
            <Show when={step.command}>{(command) => <props.CommandBox command={command()} />}</Show>
          </div>
        )}
      </For>
    </div>
  );
}

type PageState = { url: string; title: string; loading: boolean; can_back: boolean; can_forward: boolean };

/** Long-press → right click. */
const LONG_PRESS_MS = 550;
/** Movement (px) that turns a tap into a drag-scroll. */
const TAP_SLOP = 8;

/** The live view: full screen (Preview), or beside the chat (UNI-296). */
export function BrowserViewer(props: {
  hostId: string;
  hostName: string;
  url?: string;
  target?: string;
  onClose: () => void;
  CommandBox: CommandBoxComp;
  /** `screen` (default): Preview's full-screen viewer; `pane` / `sheet`: beside the chat;
   *  `tab` (WP-B8): a side-pane tab — the pane's tab strip owns the title and close. */
  variant?: 'screen' | 'pane' | 'sheet' | 'tab';
  /** UNI-296: the browser profile to show (default "default"). */
  profile?: string;
  /** UNI-296: the profile picker chose another one (the parent re-opens the view). */
  onProfile?: (name: string) => void;
  /** The shown page changed (url/title): a pane tab titles itself with it. */
  onPage?: (page: { url: string; title: string }) => void;
  /** WP-B8: the element picker is offered; a pick hands over the element + its crop. */
  onPick?: (pick: BrowserPick) => void;
  /** WP-B8: the screenshot button is offered; it hands over the current picture. */
  onCapture?: (shot: BrowserShot) => void;
  /** E-15: a browser tool of the chat is running right now (click, evaluate, wait…): the agent holds the tab. */
  agentBusy?: () => boolean;
}) {
  const embedded = () => (props.variant ?? 'screen') !== 'screen';
  const [state, setState] = createSignal<SessionState>({ kind: 'starting' });
  const [open, setOpen] = createSignal<BrowserOpenData | undefined>();
  const [tabs, setTabs] = createSignal<BrowserTab[]>([]);
  const [active, setActive] = createSignal<string | undefined>();
  const [page, setPage] = createSignal<PageState>({ url: props.url ?? '', title: '', loading: false, can_back: false, can_forward: false });
  const [address, setAddress] = createSignal(props.url ?? '');
  const [editing, setEditing] = createSignal(false);
  const [hasFrame, setHasFrame] = createSignal(false);
  const [keyboard, setKeyboard] = createSignal(false);
  const [generation, setGeneration] = createSignal(0);
  const [menu, setMenu] = createSignal<DOMRect | undefined>();
  /** undefined = unknown (older host): offer the switch, the PC answers. */
  const [canShow, setCanShow] = createSignal(true);
  const [switching, setSwitching] = createSignal(false);
  /** Reconnecting after the PC relaunched its browser (window shown/hidden). */
  const [relaunching, setRelaunching] = createSignal(false);
  const [windowError, setWindowError] = createSignal<string | undefined>();
  // UNI-296: agents + Take over, and the profile picker.
  const [agent, setAgent] = createSignal<TakeoverState>({ agent: undefined });
  const act = (action: TakeoverAction) => setAgent((s) => takeover(s, action));
  const [profiles, setProfiles] = createSignal<BrowserProfilesData | undefined>();
  const [profileMenu, setProfileMenu] = createSignal<DOMRect | undefined>();
  const profileName = () => open()?.profile ?? props.profile ?? 'default';
  let canvas!: HTMLCanvasElement;
  let wrap!: HTMLDivElement;
  let ime!: HTMLTextAreaElement;
  let session: BrowserSession | undefined;
  let frameSize = { width: 0, height: 0 };
  let viewport: BrowserViewport | undefined;
  let imeState = imeInitial();
  /** When the user last opened a tab here (its arrival is not the agent's). */
  let lastUserTab = 0;
  /** The last drawn frame (picker crop, screenshot). */
  let lastImage: ImageBitmap | HTMLImageElement | undefined;
  onCleanup(() => {
    if (lastImage && 'close' in lastImage) lastImage.close();
  });
  // WP-B8 element picker: hover box (frame px) and whether a pick is armed.
  const [picking, setPicking] = createSignal(false);
  const [hover, setHover] = createSignal<BrowserElement | undefined>();
  let inspectAt = 0;
  let inspectBusy = false;
  let pickTimer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => clearTimeout(pickTimer));
  const stopPicking = () => {
    clearTimeout(pickTimer);
    setPicking(false);
    setHover(undefined);
    inspectBusy = false;
  };
  const togglePicker = () => {
    if (picking()) return stopPicking();
    setPicking(true);
    canvas?.focus({ preventScroll: true });
  };
  function shotOf(rect?: { x: number; y: number; width: number; height: number }): BrowserShot | undefined {
    if (!lastImage || !frameSize.width) return undefined;
    const iw = 'naturalWidth' in lastImage ? lastImage.naturalWidth : lastImage.width;
    const ih = 'naturalHeight' in lastImage ? lastImage.naturalHeight : lastImage.height;
    const crop = cropRect(rect, frameSize, { width: iw, height: ih });
    if (!crop) return undefined;
    const out = document.createElement('canvas');
    out.width = crop.outWidth;
    out.height = crop.outHeight;
    const c = out.getContext('2d');
    if (!c) return undefined;
    c.drawImage(lastImage, crop.sx, crop.sy, crop.sw, crop.sh, 0, 0, crop.outWidth, crop.outHeight);
    const mime = rect ? 'image/png' : 'image/jpeg';
    const url = out.toDataURL(mime, 0.9);
    const data = url.slice(url.indexOf(',') + 1);
    return data ? { mime, data, width: crop.outWidth, height: crop.outHeight } : undefined;
  }

  // Android Back / Esc closes the full-screen viewer and the sheet; the desktop pane and a
  // pane tab stay (Esc typed into the page must not close them) — they have their own ✕.
  createEffect(() => {
    if (props.variant !== 'pane' && props.variant !== 'tab') onCleanup(pushBack(() => props.onClose()));
  });

  const mobile = () => isTauriMobile() || (typeof matchMedia !== 'undefined' && matchMedia('(pointer: coarse)').matches);

  function measure(): BrowserViewport | undefined {
    if (!wrap) return undefined;
    const box = wrap.getBoundingClientRect();
    if (box.width < 50 || box.height < 50) return undefined;
    return viewportFor(box.width, box.height, window.devicePixelRatio || 1, mobile());
  }

  async function draw(frame: Frame) {
    const blob = new Blob([frame.jpeg as BlobPart], { type: 'image/jpeg' });
    let image: ImageBitmap | HTMLImageElement;
    try {
      image = await createImageBitmap(blob);
    } catch {
      return;
    }
    const box = wrap.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const w = Math.round(box.width * dpr);
    const h = Math.round(box.height * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    frameSize = { width: frame.cssWidth, height: frame.cssHeight };
    // Fit the page into the box (letterbox while a resize is in flight).
    const scale = Math.min(box.width / frame.cssWidth, box.height / frame.cssHeight);
    const dw = frame.cssWidth * scale * dpr;
    const dh = frame.cssHeight * scale * dpr;
    ctx.fillStyle = '#111';
    ctx.fillRect(0, 0, w, h);
    ctx.drawImage(image, (w - dw) / 2, (h - dh) / 2, dw, dh);
    // Kept for the picker's crop and the screenshot button (the previous one is freed).
    if (lastImage && lastImage !== image && 'close' in lastImage) lastImage.close();
    lastImage = image;
    setHasFrame(true);
  }

  function onEvent(event: BrowserEvent) {
    if (event.kind === 'tabs') {
      const known = new Set(tabs().map((t) => t.id));
      setTabs(event.tabs);
      setActive(event.active);
      // WP-B8: a tab the agent opened (not one the user asked for) — show it, so the
      // pane follows the agent's work instead of a stale tab.
      const fresh = followTab(known, event.tabs, event.active, agent().agent, Date.now() - lastUserTab);
      if (fresh) send({ op: 'tab_activate', id: fresh });
    } else if (event.kind === 'page') {
      setPage(event);
      props.onPage?.({ url: event.url, title: event.title });
      if (!editing()) setAddress(displayUrl(event.url));
    } else if (event.kind === 'agent') {
      act({ kind: 'agent', agent: event.agent });
    } else if (event.kind === 'element') {
      inspectBusy = false;
      if (!picking()) return;
      if (!event.pick) {
        setHover(event.element);
        return;
      }
      const element = event.element;
      stopPicking();
      if (element) props.onPick?.({ element, url: page().url, title: page().title, shot: shotOf(element) });
    }
  }

  function start() {
    session?.stop();
    setHasFrame(false);
    viewport = measure() ?? viewportFor(412, 780, 2, true);
    const s: BrowserSession = new BrowserSession(props.hostId, {
      onState: (next) => {
        if (s !== session) return;
        if (next.kind === 'problem' && next.problem.kind !== 'relaunch') setRelaunching(false);
        setState(next);
      },
      onEvent: (event) => {
        if (s === session) onEvent(event);
      },
      onFrame: (frame): Promise<void> | void => (s === session ? draw(frame) : undefined),
      onOpen: (data) => {
        setRelaunching(false);
        setOpen(data);
        setTabs(data.tabs);
        setActive(data.target);
        act({ kind: 'agent', agent: data.agent });
      },
    });
    session = s;
    void s.open({ viewport, url: generation() === 0 ? props.url : undefined, target: generation() === 0 ? props.target : active(), profile: props.profile });
  }

  onMount(() => {
    start();
    void browserStatus(props.hostId).then((r) => r.ok && setCanShow(canShowWindow(r.value)));
    if (embedded()) void browserProfiles(props.hostId).then((r) => r.ok && setProfiles(r.value));
    // Rotate/resize → the PC's viewport follows (debounced).
    let timer: ReturnType<typeof setTimeout> | undefined;
    const observer = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        const next = measure();
        if (!next || sameViewport(viewport, next)) return;
        viewport = next;
        void session?.send({ op: 'viewport', viewport: next });
      }, 180);
    });
    observer.observe(wrap);
    onCleanup(() => {
      observer.disconnect();
      clearTimeout(timer);
    });
  });
  onCleanup(() => session?.stop());

  const send = (cmd: Parameters<BrowserSession['send']>[0]) => void session?.send(cmd);
  const live = () => state().kind === 'live';
  const problem = (): SessionProblem | undefined => {
    const s = state();
    return s.kind === 'problem' ? s.problem : undefined;
  };
  // The failed launch's stderr tail (UNI-287), shown behind "Details".
  const crashStderr = (): string[] | undefined => {
    const p = problem();
    return p?.kind === 'crashed' ? p.stderr : undefined;
  };

  // ------------------------------------------------------------ pointer
  const box = () => {
    const r = wrap.getBoundingClientRect();
    return { left: r.left, top: r.top, width: r.width, height: r.height };
  };
  const pagePoint = (clientX: number, clientY: number) => {
    const b = box();
    if (!frameSize.width) return undefined;
    return toPage(clientX - b.left, clientY - b.top, b, frameSize);
  };

  type Gesture = {
    id: number;
    startX: number;
    startY: number;
    lastX: number;
    lastY: number;
    moved: boolean;
    long?: ReturnType<typeof setTimeout>;
    longFired: boolean;
    mouse: boolean;
  };
  const pointers = new Map<number, { x: number; y: number }>();
  let gesture: Gesture | undefined;
  let pinchStart: { dist: number; cx: number; cy: number } | undefined;

  function onPointerDown(e: PointerEvent) {
    if (!live()) return;
    if (picking()) {
      e.preventDefault();
      const p = pagePoint(e.clientX, e.clientY);
      if (!p) return;
      send({ op: 'inspect', x: p.x, y: p.y, pick: true });
      // An older PC never answers: give the picker up rather than hang.
      clearTimeout(pickTimer);
      pickTimer = setTimeout(stopPicking, 4000);
      return;
    }
    canvas.focus({ preventScroll: true });
    canvas.setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size === 2) {
      // Second finger: switch to pinch, cancel the tap.
      if (gesture?.long) clearTimeout(gesture.long);
      gesture = undefined;
      const [a, b] = [...pointers.values()];
      pinchStart = { dist: Math.hypot(a.x - b.x, a.y - b.y), cx: (a.x + b.x) / 2, cy: (a.y + b.y) / 2 };
      return;
    }
    const p = pagePoint(e.clientX, e.clientY);
    if (!p) return;
    const mouse = e.pointerType === 'mouse';
    gesture = { id: e.pointerId, startX: e.clientX, startY: e.clientY, lastX: e.clientX, lastY: e.clientY, moved: false, longFired: false, mouse };
    if (mouse) {
      const button = e.button === 2 ? 'right' : e.button === 1 ? 'middle' : 'left';
      send({ op: 'mouse', kind: 'pressed', x: p.x, y: p.y, button, clicks: e.detail || 1, modifiers: modifiers(e) });
      return;
    }
    const g = gesture;
    g.long = setTimeout(() => {
      if (gesture !== g || g.moved) return;
      g.longFired = true;
      const q = pagePoint(g.startX, g.startY);
      if (!q) return;
      send({ op: 'mouse', kind: 'pressed', x: q.x, y: q.y, button: 'right', clicks: 1 });
      send({ op: 'mouse', kind: 'released', x: q.x, y: q.y, button: 'right', clicks: 1 });
      navigator.vibrate?.(10);
    }, LONG_PRESS_MS);
  }

  function onPointerMove(e: PointerEvent) {
    if (!live()) return;
    if (picking()) {
      // Hover box: one inspect in flight, at most ~12 per second.
      const now = performance.now();
      if (inspectBusy && now - inspectAt < 600) return;
      if (now - inspectAt < 80) return;
      const p = pagePoint(e.clientX, e.clientY);
      if (!p) return;
      inspectAt = now;
      inspectBusy = true;
      send({ op: 'inspect', x: p.x, y: p.y });
      return;
    }
    if (pointers.has(e.pointerId)) pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pinchStart && pointers.size === 2) return;
    const g = gesture;
    if (!g || g.id !== e.pointerId) {
      // Hover (desktop mouse, no button).
      if (e.pointerType === 'mouse' && !gesture) {
        const p = pagePoint(e.clientX, e.clientY);
        if (p) send({ op: 'mouse', kind: 'moved', x: p.x, y: p.y, modifiers: modifiers(e) });
      }
      return;
    }
    if (g.mouse) {
      const p = pagePoint(e.clientX, e.clientY);
      if (p) send({ op: 'mouse', kind: 'moved', x: p.x, y: p.y, button: 'left', modifiers: modifiers(e) });
      return;
    }
    const dx = e.clientX - g.lastX;
    const dy = e.clientY - g.lastY;
    if (!g.moved && Math.hypot(e.clientX - g.startX, e.clientY - g.startY) < TAP_SLOP) return;
    if (!g.moved) {
      g.moved = true;
      if (g.long) clearTimeout(g.long);
    }
    g.lastX = e.clientX;
    g.lastY = e.clientY;
    // Finger drag scrolls the page (content follows the finger).
    const p = pagePoint(g.startX, g.startY);
    const scale = frameSize.width ? frameSize.width / box().width : 1;
    if (p) send({ op: 'mouse', kind: 'wheel', x: p.x, y: p.y, dx: -dx * scale, dy: -dy * scale });
  }

  function onPointerUp(e: PointerEvent) {
    pointers.delete(e.pointerId);
    if (picking()) return;
    if (pinchStart) {
      if (pointers.size < 2) {
        const remaining = [...pointers.values()];
        const last = { x: e.clientX, y: e.clientY };
        const other = remaining[0] ?? last;
        const dist = Math.hypot(other.x - last.x, other.y - last.y);
        const scale = pinchStart.dist > 0 ? dist / pinchStart.dist : 1;
        const p = pagePoint(pinchStart.cx, pinchStart.cy);
        if (p && Math.abs(scale - 1) > 0.08) send({ op: 'pinch', x: p.x, y: p.y, scale: Math.min(4, Math.max(0.25, scale)) });
        pinchStart = undefined;
      }
      return;
    }
    const g = gesture;
    if (!g || g.id !== e.pointerId) return;
    gesture = undefined;
    if (g.long) clearTimeout(g.long);
    if (!live()) return;
    const p = pagePoint(e.clientX, e.clientY) ?? pagePoint(g.startX, g.startY);
    if (!p) return;
    if (g.mouse) {
      const button = e.button === 2 ? 'right' : e.button === 1 ? 'middle' : 'left';
      send({ op: 'mouse', kind: 'released', x: p.x, y: p.y, button, clicks: e.detail || 1, modifiers: modifiers(e) });
      return;
    }
    if (!g.moved && !g.longFired) {
      const q = pagePoint(g.startX, g.startY) ?? p;
      send({ op: 'mouse', kind: 'moved', x: q.x, y: q.y });
      send({ op: 'mouse', kind: 'pressed', x: q.x, y: q.y, button: 'left', clicks: 1 });
      send({ op: 'mouse', kind: 'released', x: q.x, y: q.y, button: 'left', clicks: 1 });
    }
  }

  function onWheel(e: WheelEvent) {
    if (!live()) return;
    if (picking()) setHover(undefined);
    e.preventDefault();
    const p = pagePoint(e.clientX, e.clientY);
    if (!p) return;
    const unit = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? box().height : 1;
    send({ op: 'mouse', kind: 'wheel', x: p.x, y: p.y, dx: e.deltaX * unit, dy: e.deltaY * unit, modifiers: modifiers(e) });
  }

  // ------------------------------------------------------------ keyboard
  function onCanvasKey(e: KeyboardEvent) {
    if (!live()) return;
    if (picking() && e.key === 'Escape') {
      e.preventDefault();
      stopPicking();
      return;
    }
    const cmd = keyCommand(e);
    if (!cmd) return;
    e.preventDefault();
    send(cmd);
  }
  function onImeKey(e: KeyboardEvent) {
    // Specials the soft keyboard reports as keydown (Enter, Backspace on an empty field, arrows).
    if (e.key === 'Enter' || (e.key === 'Backspace' && ime.value === '') || (e.key.startsWith('Arrow') && ime.value === '')) {
      e.preventDefault();
      send({ op: 'key', kind: 'press', key: e.key, code: e.code || e.key, modifiers: modifiers(e) });
      return;
    }
    if (e.ctrlKey || e.altKey || e.metaKey || e.key === 'Tab' || e.key === 'Escape') {
      const cmd = keyCommand(e);
      if (cmd) {
        e.preventDefault();
        send(cmd);
      }
    }
  }
  function onImeInput() {
    const result = imeCommands(ime.value, imeState);
    imeState = result.state;
    if (result.resetValue !== undefined) ime.value = result.resetValue;
    for (const cmd of result.cmds) send(cmd);
  }
  function toggleKeyboard() {
    if (keyboard()) {
      ime.blur();
      setKeyboard(false);
      return;
    }
    ime.value = '';
    imeState = imeInitial();
    setKeyboard(true);
    ime.focus({ preventScroll: true });
  }

  // ------------------------------------------------------------ chrome
  const go = (e: Event) => {
    e.preventDefault();
    const text = address().trim();
    if (!text) return;
    setEditing(false);
    (document.activeElement as HTMLElement | null)?.blur?.();
    send({ op: 'navigate', url: text });
  };
  const retry = () => {
    setGeneration((g) => g + 1);
    start();
  };
  // The PC relaunched its browser to show/hide the window: reconnect by ourselves.
  createEffect(() => {
    if (problem()?.kind !== 'relaunch') return;
    setRelaunching(true);
    const timer = setTimeout(retry, 400);
    onCleanup(() => clearTimeout(timer));
  });
  // UNI-299: a host that reports `display` knows hidden vs window (CloakBrowser
  // is headed on a hidden display by default); older hosts only have `headless`.
  const shown = () => !!open() && windowShown(open()!);
  async function setTakeover(on: boolean) {
    act({ kind: 'request', on });
    const r = await browserTakeover(props.hostId, on, open()?.profile ?? props.profile);
    act(r.ok ? { kind: 'ok', agent: r.value } : { kind: 'failed', error: r.error });
  }
  const strip = (): ReturnType<typeof agentStrip> => {
    const base = agentStrip(agent().agent);
    return base.kind === 'none' && props.agentBusy?.() && !agent().agent?.taken_over ? { kind: 'driving' } : base;
  };
  const badge = () => stealthBadge(open());
  async function toggleWindow() {
    setMenu(undefined);
    const show = !shown();
    setSwitching(true);
    setWindowError(undefined);
    const r = await browserWindow(props.hostId, show);
    setSwitching(false);
    if (!r.ok) {
      const old = r.error.code === 'E_SCHEMA' || r.error.code === 'E_UNSUPPORTED';
      setWindowError(old ? 'Update UniPi on the PC to show the browser there.' : r.error.message);
      return;
    }
    setCanShow(canShowWindow(r.value));
    // Already in that mode (nothing relaunched): reflect it right away.
    setOpen((o) => (o ? { ...o, headless: r.value.headless, display: r.value.display ?? (show ? 'window' : 'hidden') } : o));
  }

  // Header pieces (the full-screen header and the pane's URL row share them).
  const navBack = () => (
    <button type="button" class="ghost-icon-button browser-nav" aria-label="Page back" disabled={!live() || !page().can_back} onClick={() => send({ op: 'back' })}>
      <ArrowLeft size={17} />
    </button>
  );
  const navForward = () => (
    <button type="button" class="ghost-icon-button browser-nav browser-fwd" aria-label="Page forward" disabled={!live() || !page().can_forward} onClick={() => send({ op: 'forward' })}>
      <ArrowRight size={17} />
    </button>
  );
  const urlBar = () => (
    <form class="preview-urlbar" onSubmit={go}>
      <span class="browser-url-origin" title={`${browserName(open()?.browser.name)} on ${props.hostName}`}>
        <Globe size={13} />
      </span>
      <input
        class="preview-url-input"
        aria-label="Address"
        value={address()}
        placeholder="Search or type a URL"
        autocapitalize="off"
        autocorrect="off"
        spellcheck={false}
        enterkeyhint="go"
        inputmode="url"
        disabled={!live()}
        onFocus={(e) => {
          setEditing(true);
          e.currentTarget.select();
        }}
        onBlur={() => {
          setEditing(false);
          setAddress(displayUrl(page().url));
        }}
        onInput={(e) => setAddress(e.currentTarget.value)}
      />
    </form>
  );
  const reloadButton = () => (
    <button
      type="button"
      class="ghost-icon-button"
      aria-label={page().loading ? 'Stop loading' : 'Reload'}
      disabled={!live()}
      onClick={() => send({ op: page().loading ? 'stop' : 'reload' })}
    >
      <Show when={page().loading} fallback={<RotateCw size={17} />}>
        <Square size={14} />
      </Show>
    </button>
  );
  const keyboardButton = () => (
    <button type="button" class="ghost-icon-button" aria-label={keyboard() ? 'Hide keyboard' : 'Type'} aria-pressed={keyboard()} disabled={!live()} onClick={toggleKeyboard}>
      <Keyboard size={18} />
    </button>
  );
  const pickerButton = () => (
    <Show when={props.onPick}>
      <button
        type="button"
        class="ghost-icon-button browser-tool"
        aria-label={picking() ? 'Stop picking' : 'Pick an element for the chat'}
        title={picking() ? 'Stop picking (Esc)' : 'Pick an element for the chat'}
        aria-pressed={picking()}
        data-testid="browser-picker"
        disabled={!live() || !hasFrame()}
        onClick={togglePicker}
      >
        <SquareMousePointer size={17} />
      </button>
    </Show>
  );
  const captureButton = () => (
    <Show when={props.onCapture}>
      <button
        type="button"
        class="ghost-icon-button browser-tool"
        aria-label="Screenshot to the chat"
        title="Screenshot to the chat"
        data-testid="browser-capture"
        disabled={!live() || !hasFrame()}
        onClick={() => {
          const shot = shotOf();
          if (shot) props.onCapture?.(shot);
        }}
      >
        <Camera size={17} />
      </button>
    </Show>
  );
  const menuButton = () => (
    <button
      type="button"
      class="ghost-icon-button"
      aria-label="Browser menu"
      aria-haspopup="menu"
      aria-expanded={!!menu()}
      data-testid="browser-menu-button"
      onClick={(e) => setMenu(menu() ? undefined : e.currentTarget.getBoundingClientRect())}
    >
      <Show when={switching()} fallback={<EllipsisVertical size={18} />}>
        <RefreshCw size={16} class="refresh-spinning" />
      </Show>
    </button>
  );

  // The pane/tab header pieces. `compact`: the tab toolbar — the badge is an icon with
  // the full wording in its tooltip; the profile chip hides while there is only one.
  const profileChip = (compact: boolean) => (
    <Show when={!compact || canPickProfile(profiles())}>
      <button
        type="button"
        class="browser-profile-chip"
        data-testid="browser-profile-chip"
        aria-haspopup={canPickProfile(profiles()) ? 'menu' : undefined}
        disabled={!canPickProfile(profiles())}
        title={canPickProfile(profiles()) ? `Browser profile ${profileName()} — switch` : `Browser profile ${profileName()}`}
        onClick={(e) => setProfileMenu(profileMenu() ? undefined : e.currentTarget.getBoundingClientRect())}
      >
        <span class="truncate">{profileName()}</span>
        <Show when={canPickProfile(profiles())}>
          <ChevronDown size={14} />
        </Show>
      </button>
    </Show>
  );
  const badgeChip = (compact: boolean) => (
    <Show when={badge()}>
      {(b) => (
        <span
          class="browser-badge"
          classList={{ 'browser-badge-icon': compact }}
          data-tone={b().tone}
          title={b().title}
          aria-label={compact ? b().label : undefined}
          role={compact ? 'img' : undefined}
          data-testid="browser-stealth-badge"
        >
          <Show when={b().tone === 'ok'} fallback={<ShieldAlert size={13} />}>
            <ShieldCheck size={13} />
          </Show>
          <Show when={!compact}>{b().label}</Show>
        </span>
      )}
    </Show>
  );
  const stealthNote = () => (
    <Show when={badge()?.note}>
      {(note) => (
        <p class="browser-stealth-note" data-testid="browser-stealth-note">
          <ShieldAlert size={13} /> <span>{note()}</span>
        </p>
      )}
    </Show>
  );

  return (
    <div
      class={embedded() ? 'browser-viewer browser-embedded' : 'preview-viewer browser-viewer'}
      data-variant={props.variant ?? 'screen'}
      data-picking={picking() ? '' : undefined}
      data-testid="browser-viewer"
      data-state={state().kind}
    >
      <Switch>
        <Match when={!embedded()}>
          <header class="preview-viewer-head browser-head detail-head">
            <button type="button" class="ghost-icon-button" aria-label="Back to Preview" onClick={props.onClose}>
              <X size={20} />
            </button>
            {navBack()}
            {navForward()}
            {urlBar()}
            {reloadButton()}
            {keyboardButton()}
            {menuButton()}
          </header>
        </Match>
        <Match when={props.variant === 'tab'}>
          {/* WP-B8: one toolbar row — the side pane's strip already names and closes the tab. */}
          <div class="preview-viewer-head browser-head browser-tab-head" data-testid="browser-tab-head">
            {navBack()}
            {navForward()}
            {reloadButton()}
            {urlBar()}
            <Show when={mobile()}>{keyboardButton()}</Show>
            {pickerButton()}
            {captureButton()}
            {profileChip(true)}
            {badgeChip(true)}
            {menuButton()}
          </div>
          {stealthNote()}
        </Match>
        <Match when={embedded()}>
          <header class="browser-pane-head detail-head" data-testid="browser-pane-head">
            <span class="browser-pane-title">
              <Globe size={16} />
              <span>Browser</span>
            </span>
            {profileChip(false)}
            {badgeChip(false)}
            <span class="browser-pane-spacer" />
            {menuButton()}
            <button type="button" class="ghost-icon-button" aria-label="Close the browser" data-testid="browser-pane-close" onClick={props.onClose}>
              <X size={19} />
            </button>
          </header>
          <div class="preview-viewer-head browser-head browser-pane-nav">
            {navBack()}
            {navForward()}
            {urlBar()}
            {reloadButton()}
            <Show when={mobile()}>{keyboardButton()}</Show>
          </div>
          {stealthNote()}
        </Match>
      </Switch>
      <Show when={profileMenu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} label="Browser profile" width={260} onDismiss={() => setProfileMenu(undefined)}>
            <MenuLabel>Profiles on {props.hostName}</MenuLabel>
            <For each={profileChoices(profiles(), profileName())}>
              {(choice) => (
                <MenuRow
                  icon={choice.current ? <Check size={18} /> : <span class="browser-profile-blank" />}
                  label={choice.name}
                  hint={choice.hint ? <span class="board-menu-note">{choice.hint}</span> : undefined}
                  onSelect={() => {
                    setProfileMenu(undefined);
                    if (!choice.current) props.onProfile?.(choice.name);
                  }}
                />
              )}
            </For>
          </AnchoredMenu>
        )}
      </Show>
      <Show when={strip().kind !== 'none'}>
        <div class="browser-agent-strip" data-kind={strip().kind} role="status" data-testid="browser-agent-strip">
          <Show
            when={strip().kind === 'taken'}
            fallback={
              <>
                <span class="browser-agent-dot" aria-hidden="true" />
                <span class="browser-agent-text">Agent is using the browser</span>
                <button type="button" class="btn btn-sm btn-primary browser-agent-btn" data-testid="browser-takeover" disabled={!!agent().pending} onClick={() => void setTakeover(true)}>
                  <Show when={agent().pending === 'take'} fallback={<Hand size={15} />}>
                    <RefreshCw size={14} class="refresh-spinning" />
                  </Show>
                  Take over
                </button>
              </>
            }
          >
            <Hand size={16} class="browser-agent-hand" />
            <span class="browser-agent-text">
              You have control<span class="browser-agent-sub"> · the agent waits</span>
            </span>
            <button type="button" class="btn btn-sm btn-secondary browser-agent-btn" data-testid="browser-giveback" disabled={!!agent().pending} onClick={() => void setTakeover(false)}>
              <Show when={agent().pending === 'give'}>
                <RefreshCw size={14} class="refresh-spinning" />
              </Show>
              Give back
            </button>
          </Show>
        </div>
      </Show>
      <Show when={agent().error}>
        <p class="browser-headed-note browser-window-error" role="alert" data-testid="browser-takeover-error">{agent().error}</p>
      </Show>
      <Show when={menu()}>
        {(anchor) => (
          <AnchoredMenu anchor={anchor()} label="Browser menu" alignEnd width={260} onDismiss={() => setMenu(undefined)}>
            <Show when={canShow()}>
              <MenuRow
                icon={shown() ? <EyeOff size={18} /> : <AppWindow size={18} />}
                label={shown() ? 'Hide the window (app only)' : `Show window on ${props.hostName}`}
                disabled={!live() || switching()}
                onSelect={() => void toggleWindow()}
              />
              <MenuSeparator />
            </Show>
            <MenuRow
              icon={<Power size={18} />}
              label={`Quit the browser on ${props.hostName}`}
              tone="danger"
              onSelect={() => {
                setMenu(undefined);
                void browserClose(props.hostId);
              }}
            />
          </AnchoredMenu>
        )}
      </Show>
      <div class="browser-tabs" role="tablist" aria-label="Tabs on the PC" data-testid="browser-tabstrip">
        <For each={tabs()}>
          {(tab) => (
            <div class="browser-tab" role="tab" aria-selected={tab.id === active()} data-active={tab.id === active() ? 'true' : undefined}>
              <button type="button" class="browser-tab-main" title={tab.url} disabled={!live()} onClick={() => tab.id !== active() && send({ op: 'tab_activate', id: tab.id })}>
                {tabLabel(tab)}
              </button>
              <button type="button" class="browser-tab-close" aria-label={`Close tab ${tabLabel(tab)}`} disabled={!live()} onClick={() => send({ op: 'tab_close', id: tab.id })}>
                <X size={13} />
              </button>
            </div>
          )}
        </For>
        <button
          type="button"
          class="browser-tab-new"
          aria-label="New tab"
          title="New tab"
          disabled={!live()}
          onClick={() => {
            lastUserTab = Date.now();
            send({ op: 'tab_new' });
          }}
        >
          <Plus size={16} />
        </button>
      </div>
      <Show when={page().loading && live()}>
        <div class="browser-progress" aria-hidden="true" />
      </Show>
      <div class="browser-canvas-wrap" ref={wrap}>
        <canvas
          ref={canvas}
          class="browser-canvas"
          tabindex="0"
          aria-label={`Browser on ${props.hostName}: ${page().title || displayUrl(page().url) || 'page'}`}
          data-testid="browser-canvas"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onWheel={onWheel}
          onKeyDown={onCanvasKey}
          onContextMenu={(e) => e.preventDefault()}
        />
        <textarea
          ref={ime}
          class="browser-ime"
          aria-label="Type into the page"
          autocapitalize="off"
          autocomplete="off"
          spellcheck={false}
          onInput={onImeInput}
          onKeyDown={onImeKey}
          onBlur={() => setKeyboard(false)}
        />
        <Show when={picking()}>
          <div class="browser-pick-hint" role="status" data-testid="browser-pick-hint">
            Click an element to add it to the chat · Esc cancels
          </div>
        </Show>
        <Show when={picking() && hover()}>
          {(el) => {
            const at = () => {
              const b = box();
              return overlayRect(el(), { width: b.width, height: b.height }, frameSize);
            };
            return (
              <div class="browser-pick-box" data-testid="browser-pick-box" style={{ left: `${at().x}px`, top: `${at().y}px`, width: `${at().width}px`, height: `${at().height}px` }}>
                <span class="browser-pick-label" data-below={at().y < 22 ? '' : undefined}>
                  {el().label}
                </span>
              </div>
            );
          }}
        </Show>
        <Show when={state().kind === 'starting' || (live() && !hasFrame()) || problem()?.kind === 'relaunch'}>
          <div class="browser-overlay" data-testid="browser-starting">
            <RefreshCw size={22} class="refresh-spinning" />
            <p>
              {problem()?.kind === 'relaunch' || relaunching()
                ? `Switching the window on ${props.hostName}…`
                : state().kind === 'starting'
                  ? `Starting the browser on ${props.hostName}…`
                  : 'Waiting for the first picture…'}
            </p>
          </div>
        </Show>
        <Show when={problem()?.kind !== 'relaunch' && problem()}>
          {(p) => (
            <div class="browser-overlay browser-problem" data-kind={p().kind} role="alert" data-testid="browser-problem">
              <span class="browser-problem-icon">
                <Show when={p().kind === 'disabled' || p().kind === 'lost'} fallback={<TriangleAlert size={22} />}>
                  <Unplug size={22} />
                </Show>
              </span>
              <h2>{problemTitle(p())}</h2>
              <Show when={p().message !== problemTitle(p())}>
                <p>{p().message}</p>
              </Show>
              <Show when={p().kind === 'disabled'}>
                <props.CommandBox command={ENABLE_BROWSER_COMMAND} />
              </Show>
              <Show when={p().kind === 'not-found'}>
                <InstallChrome CommandBox={props.CommandBox} />
              </Show>
              <Show when={crashStderr()}>
                {(lines) => (
                  <details class="browser-problem-details">
                    <summary>Details</summary>
                    <pre data-testid="browser-problem-stderr">{lines().join('\n')}</pre>
                  </details>
                )}
              </Show>
              <div class="browser-problem-actions">
                <button type="button" class="preview-btn primary" onClick={retry}>
                  <RefreshCw size={16} /> {p().kind === 'crashed' || p().kind === 'closed' ? 'Restart' : 'Try again'}
                </button>
                <button type="button" class="preview-btn" onClick={props.onClose}>
                  Back
                </button>
              </div>
            </div>
          )}
        </Show>
      </div>
      <Show when={windowError()}>
        <p class="browser-headed-note browser-window-error" role="alert">{windowError()}</p>
      </Show>
      <Show when={!windowError() && shown()}>
        <p class="browser-headed-note" data-testid="browser-headed-note">Also showing in a window on {props.hostName}</p>
      </Show>
    </div>
  );
}

export function problemTitle(p: SessionProblem): string {
  switch (p.kind) {
    case 'disabled':
      return 'The remote browser is off on this PC';
    case 'not-found':
      return "Google Chrome isn't installed on the PC";
    case 'crashed':
      return 'The browser crashed';
    case 'closed':
      return 'The browser was closed';
    case 'relaunch':
      return 'Switching the window on the PC';
    case 'lost':
      return 'Connection lost';
  }
}
