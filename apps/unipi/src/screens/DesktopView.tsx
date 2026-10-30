/**
 * Preview → Desktop (UNI-216, RustDesk-style UX UNI-271): watch and drive the PC's whole screen.
 * `DesktopPanel` is the pre-start card (off on the PC / no display / not
 * compiled / ready, with consent and view-only notes); `DesktopViewer` is
 * the full-screen live view: H.264 frames decoded with WebCodecs onto a
 * canvas, a small top bar (back, PC name, view-only badge, quality, fit/1:1,
 * keyboard) and an optional key row (Esc Tab Ctrl Alt Shift Super arrows…).
 *
 * Touch (phone): tap = click, long-press = right click, two-finger tap =
 * right click, drag = left drag, two-finger drag = scroll, pinch = zoom the
 * picture locally (and pan with two fingers while zoomed). Mouse and
 * keyboard (desktop app): map directly (buttons, wheel, every key down/up).
 */
import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ChevronDown,
  Command,
  Eye,
  Gauge,
  Hand,
  Keyboard,
  Maximize,
  Minimize,
  Monitor,
  MonitorCog,
  MonitorPlay,
  MousePointer2,
  Pin,
  PinOff,
  RefreshCw,
  Expand,
  ShieldQuestion,
  TriangleAlert,
  Unplug,
  X,
} from 'lucide-solid';
import type { DesktopQuality, DesktopStatusData } from '@unipi/protocol';
import type { Result } from '../lib/ipc';
import { pushBack } from '../lib/back';
import { diffImeValue, imeInitial } from '../lib/ime';
import { isTauriMobile } from '../lib/platform';
import {
  displayLabel,
  ENABLE_DESKTOP_COMMAND,
  ENABLE_DESKTOP_VIEW_COMMAND,
  GestureMachine,
  LONG_PRESS_MS,
  ScreenSession,
  fitView,
  hardwareKey,
  imeToCommands,
  keysymFor,
  oneToOneView,
  pressCommands,
  problemTitle,
  screenStatus,
  statusView,
  toScreen,
  viewportFor,
  webCodecsSupport,
  avcDescription,
  annexBToAvc,
  sameBytes,
  zoomAt,
  clampView,
  KEYSYMS,
  type Frame,
  type GestureAction,
  type Modifier,
  type Problem,
  type SessionState,
  type View,
} from '../lib/screen';
import {
  chordCommands,
  drawRect,
  edgePan,
  followCursor,
  hasFeature,
  HeldKeys,
  initialQuality,
  isEscapeHatch,
  isFit,
  loadPrefs,
  originalScale,
  qualityLabel,
  qualityOptions,
  savePrefs,
  SPECIAL_KEYS,
  TOOLBAR_HIDE_MS,
  TrackpadMachine,
  viewportForView,
  zoomPercent,
  type MouseMode,
  type PadAction,
  type SpecialKey,
  type ViewStyle,
} from '../lib/desktopUx';
import { ErrorBanner } from '../components/ErrorBanner';
import { FeatureOff } from '../components/FeatureOff';
import { LeaveButton, LeaveSheet } from '../components/LeaveSheet';
import { isUnreachable, OfflineState } from '../components/EmptyState';

type CommandBoxComp = (props: { command: string }) => JSX.Element;
type NoticeComp = (props: { icon: JSX.Element; title: string; tone?: 'warn' | 'error'; testId?: string; children: JSX.Element }) => JSX.Element;

function reasonText(reason: string): { title: string; body: string } {
  switch (reason) {
    case 'not_compiled':
      return { title: "This unipi-host can't stream the desktop", body: 'It was built without desktop streaming.' };
    case 'no_display':
      return { title: 'No desktop session on the PC', body: 'unipi-host is not running inside a logged-in desktop (it looks headless, or runs over SSH only).' };
    default:
      return { title: 'Desktop streaming is not available', body: reason };
  }
}

/** The pre-start card in the Preview page's Desktop mode. */
export function DesktopPanel(props: {
  hostId: string;
  hostName: string;
  Notice: NoticeComp;
  CommandBox: CommandBoxComp;
  onOpen: (args: { viewOnly: boolean; status?: DesktopStatusData }) => void;
}) {
  const [status, setStatus] = createSignal<Result<DesktopStatusData> | undefined>();
  const [refreshing, setRefreshing] = createSignal(false);
  const [codecs, setCodecs] = createSignal<boolean | undefined>();
  const [watchOnly, setWatchOnly] = createSignal(false);
  const [leaving, setLeaving] = createSignal(false);
  async function refresh() {
    const id = props.hostId;
    setRefreshing(true);
    const result = await screenStatus(id);
    setRefreshing(false);
    if (id === props.hostId) setStatus(result);
  }
  createEffect(() => {
    props.hostId;
    setStatus(undefined);
    void refresh();
  });
  onMount(() => void webCodecsSupport().then(setCodecs));
  const view = () => statusView(status());
  const { Notice, CommandBox } = props;
  const check = () => (
    <button type="button" class="preview-btn" onClick={() => void refresh()}>
      <RefreshCw size={16} class={refreshing() ? 'refresh-spinning' : ''} /> Check again
    </button>
  );

  return (
    <div class="browser-panel" data-testid="desktop-panel">
      <Show when={codecs() === false}>
        <Notice icon={<TriangleAlert size={22} />} title="This device can't play the desktop stream" tone="error" testId="desktop-no-webcodecs">
          <p>
            Desktop streaming decodes H.264 video with WebCodecs, which this app's web view doesn't offer. Update Android
            System WebView (or Chrome) from the Play Store, then try again.
          </p>
        </Notice>
      </Show>
      <Show when={view().kind === 'loading'}>
        <div class="preview-loading" data-testid="desktop-loading">
          <RefreshCw size={18} class="refresh-spinning" /> Checking {props.hostName}'s desktop…
        </div>
      </Show>
      <Show when={view().kind === 'disabled'}>
        <div data-testid="desktop-disabled">
          <FeatureOff feature="desktop" hostId={props.hostId} hostName={props.hostName} CommandBox={CommandBox} onChanged={() => void refresh()} />
        </div>
      </Show>
      <Show when={view().kind === 'unavailable' && (view() as { reason: string; hint?: string })}>
        {(v) => (
          <Notice icon={<Monitor size={22} />} title={reasonText(v().reason).title} tone="warn" testId={`desktop-unavailable-${v().reason}`}>
            <p>{reasonText(v().reason).body}</p>
            <Show when={v().hint}>
              <p class="preview-muted">{v().hint}</p>
            </Show>
            {check()}
          </Notice>
        )}
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
      <Show when={view().kind === 'ready' && (view() as { status: DesktopStatusData }).status}>
        {(s) => (
          <>
            <div class="desktop-ready" data-testid="desktop-ready">
              <div class="desktop-ready-art" aria-hidden="true">
                <Monitor size={40} />
              </div>
              <p class="desktop-ready-title">{props.hostName}'s screen</p>
              <p class="preview-muted">
                {displayLabel(s())} ·{' '}
                {s().view_only ? 'watch only (set on the PC)' : 'mouse and keyboard'}
              </p>
              <Show when={!s().view_only}>
                <label class="desktop-check">
                  <input type="checkbox" checked={watchOnly()} onChange={(e) => setWatchOnly(e.currentTarget.checked)} /> Watch only
                  (don't send taps or keys)
                </label>
              </Show>
              <button
                type="button"
                class="preview-btn primary desktop-start"
                data-testid="desktop-start"
                disabled={codecs() === false}
                onClick={() => props.onOpen({ viewOnly: s().view_only || watchOnly(), status: s() })}
              >
                <MonitorPlay size={17} /> Show desktop
              </button>
              {/* UNI-253: a phone can stop using a feature it has. */}
              <LeaveButton onClick={() => setLeaving(true)} testId="desktop-leave" />
            </div>
            <LeaveSheet open={leaving()} onClose={() => setLeaving(false)} hostId={props.hostId} feature="desktop" allowed />
            <Show when={s().consent === 'needed'}>
              <div class="desktop-consent-note" data-testid="desktop-consent-note">
                <ShieldQuestion size={15} />
                <Show
                  when={s().display === 'macos'}
                  fallback={
                    <p>
                      The first time, {props.hostName} shows a "Share your screen" dialog. Someone at the PC has to choose{' '}
                      <b>Share</b>; it is remembered after that.
                    </p>
                  }
                >
                  <p data-testid="desktop-consent-mac">
                    {props.hostName} has to allow unipi-host once in System Settings → Privacy &amp; Security (
                    <b>Screen &amp; System Audio Recording</b>, and <b>Accessibility</b> for mouse and keyboard).
                    <Show when={s().hint}>
                      {' '}
                      <span class="preview-muted">{s().hint}</span>
                    </Show>
                  </p>
                </Show>
              </div>
            </Show>
            <p class="preview-muted browser-note">
              Everything goes over the same encrypted connection as your terminals. The PC logs every session (
              <code>unipi-host desktop status</code>). One device watches at a time.
            </p>
          </>
        )}
      </Show>
    </div>
  );
}

const KEY_ROW: { label: string; aria: string; keysym?: number; mod?: Modifier; icon?: () => JSX.Element }[] = [
  { label: 'Esc', aria: 'Escape', keysym: KEYSYMS.Escape },
  { label: 'Tab', aria: 'Tab', keysym: KEYSYMS.Tab },
  { label: 'Ctrl', aria: 'Ctrl modifier', mod: 'ctrl' },
  { label: 'Alt', aria: 'Alt modifier', mod: 'alt' },
  { label: 'Shift', aria: 'Shift modifier', mod: 'shift' },
  { label: 'Super', aria: 'Super (Windows) key', mod: 'super' },
  { label: '', aria: 'Arrow left', keysym: KEYSYMS.ArrowLeft, icon: () => <ArrowLeft size={16} /> },
  { label: '', aria: 'Arrow up', keysym: KEYSYMS.ArrowUp, icon: () => <ArrowUp size={16} /> },
  { label: '', aria: 'Arrow down', keysym: KEYSYMS.ArrowDown, icon: () => <ArrowDown size={16} /> },
  { label: '', aria: 'Arrow right', keysym: KEYSYMS.ArrowRight, icon: () => <ArrowRight size={16} /> },
  { label: 'Del', aria: 'Delete', keysym: KEYSYMS.Delete },
  { label: 'Home', aria: 'Home', keysym: KEYSYMS.Home },
  { label: 'End', aria: 'End', keysym: KEYSYMS.End },
  { label: 'PgUp', aria: 'Page up', keysym: KEYSYMS.PageUp },
  { label: 'PgDn', aria: 'Page down', keysym: KEYSYMS.PageDown },
  { label: 'F5', aria: 'F5', keysym: KEYSYMS.F5 },
  { label: 'F11', aria: 'F11', keysym: KEYSYMS.F11 },
];

type DecoderLike = {
  state: string;
  configure(c: unknown): void;
  decode(chunk: unknown): void;
  close(): void;
  decodeQueueSize?: number;
};

type Menu = 'quality' | 'view' | 'keys' | 'display' | undefined;

/**
 * The full-screen live view, RustDesk-style (UNI-271): the picture fills the
 * screen; a collapsible toolbar (pin / auto-hide, a handle to bring it back)
 * holds display picker, view style (Scale adaptive / Original 1:1), quality
 * preset, special keys (Ctrl+Alt+Del…), keyboard, mouse mode (phone),
 * fullscreen and close. Phone: touch mode (tap where you point) or mouse
 * mode (trackpad with a cursor and left/right buttons); pinch zoom, pan,
 * two-finger scroll, long-press right click. Desktop app: real mouse and
 * keyboard while the picture has focus (Ctrl+Shift+F12 releases).
 */
export function DesktopViewer(props: {
  hostId: string;
  hostName: string;
  viewOnly: boolean;
  status?: DesktopStatusData;
  onClose: () => void;
  CommandBox: CommandBoxComp;
}) {
  const prefs = loadPrefs();
  const phone = () => isTauriMobile() || (typeof matchMedia !== 'undefined' && matchMedia('(pointer: coarse)').matches);
  const [state, setState] = createSignal<SessionState>({ kind: 'connecting' });
  const [hasFrame, setHasFrame] = createSignal(false);
  const [quality, setQuality] = createSignal<DesktopQuality>(initialQuality(props.status, prefs.quality));
  const [menu, setMenu] = createSignal<Menu>();
  const [keyboard, setKeyboard] = createSignal(false);
  const [mods, setMods] = createSignal<Modifier[]>([]);
  const [style, setStyle] = createSignal<ViewStyle>(prefs.viewStyle);
  const [zoomPct, setZoomPct] = createSignal(100);
  const [mouseMode, setMouseMode] = createSignal<MouseMode>(prefs.mouseMode ?? 'touch');
  const [pinned, setPinned] = createSignal(prefs.pinned);
  const [barShown, setBarShown] = createSignal(true);
  const [fullscreen, setFullscreen] = createSignal(false);
  const [captured, setCaptured] = createSignal(false);
  const [display, setDisplay] = createSignal<string | undefined>();
  const [cursor, setCursor] = createSignal({ x: 0.5, y: 0.5 });
  const [stats, setStats] = createSignal<{ fps: number; kbps: number } | undefined>();
  const [picSize, setPicSize] = createSignal({ width: 0, height: 0 });
  const [attempt, setAttempt] = createSignal(0);
  const [toast, setToast] = createSignal<string | undefined>();
  let canvas!: HTMLCanvasElement;
  let wrap!: HTMLDivElement;
  let ime!: HTMLTextAreaElement;
  let session: ScreenSession | undefined;
  let decoder: DecoderLike | undefined;
  let decoderCodec = '';
  let screen = { width: 0, height: 0 };
  let picture = { width: 0, height: 0 };
  let view: View = { scale: 1, x: 0, y: 0 };
  let lastBitmap: ImageBitmap | VideoFrame | undefined;
  let imeState = imeInitial();
  let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let viewportTimer: ReturnType<typeof setTimeout> | undefined;
  let edgeTimer: ReturnType<typeof setInterval> | undefined;
  let lastMouse: { x: number; y: number } | undefined;
  const held = new HeldKeys();
  const gestures = new GestureMachine(() => !isFit(view, boxSize(), screen) && view.scale > fitView(boxSize(), screen).scale + 1e-6);
  const pad = new TrackpadMachine(() => ({ scale: view.scale, screen }));
  let longTimer: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => onCleanup(pushBack(() => props.onClose())));
  createEffect(() => savePrefs({ quality: quality(), viewStyle: style() === 'original' ? 'original' : 'adaptive', mouseMode: mouseMode(), pinned: pinned() }));

  const options = () => qualityOptions(props.status);
  const displays = () => props.status?.displays ?? [];
  const canPickDisplay = () => hasFeature(props.status, 'displays') && displays().length > 1;
  const live = () => state().kind === 'live';
  const dpr = () => window.devicePixelRatio || 1;
  const viewOnly = () => {
    const s = state();
    return props.viewOnly || (s.kind === 'live' && s.viewOnly);
  };
  const problem = (): Problem | undefined => {
    const s = state();
    return s.kind === 'problem' ? s.problem : undefined;
  };
  const boxSize = () => {
    const r = wrap.getBoundingClientRect();
    return { width: r.width, height: r.height };
  };
  const padMode = () => phone() && mouseMode() === 'mouse' && !viewOnly();

  function flash(text: string) {
    setToast(text);
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => setToast(undefined), 1800);
  }

  // ------------------------------------------------------------ toolbar
  function showBar() {
    setBarShown(true);
    scheduleHide();
  }
  function scheduleHide() {
    clearTimeout(hideTimer);
    if (pinned() || menu() || !live() || !hasFrame()) return;
    hideTimer = setTimeout(() => {
      if (!pinned() && !menu()) setBarShown(false);
    }, TOOLBAR_HIDE_MS);
  }
  createEffect(() => {
    // Re-evaluate when these change.
    pinned();
    menu();
    if (live() && hasFrame()) scheduleHide();
    else setBarShown(true);
  });
  const toggleMenu = (m: Exclude<Menu, undefined>) => setMenu((cur) => (cur === m ? undefined : m));

  // ------------------------------------------------------------ drawing
  function paint() {
    if (!lastBitmap || !wrap) return;
    const box = boxSize();
    const d = dpr();
    const w = Math.round(box.width * d);
    const h = Math.round(box.height * d);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.fillStyle = '#0b0b0c';
    ctx.fillRect(0, 0, w, h);
    const r = drawRect(view, screen, d);
    // Only smooth when the picture is resampled; a 1:1 picture of a 1:1
    // stream is drawn pixel for pixel.
    const pictureIsScreen = picture.width === screen.width;
    ctx.imageSmoothingEnabled = !(pictureIsScreen && !r.smooth);
    ctx.imageSmoothingQuality = 'high';
    ctx.drawImage(lastBitmap as CanvasImageSource, r.x, r.y, r.w, r.h);
    if (padMode()) drawCursor(ctx, d);
  }

  /** Mouse mode: the PC draws its own cursor into the picture, we draw a ring where taps land. */
  function drawCursor(ctx: CanvasRenderingContext2D, d: number) {
    const c = cursor();
    const x = (view.x + c.x * screen.width * view.scale) * d;
    const y = (view.y + c.y * screen.height * view.scale) * d;
    ctx.save();
    ctx.lineWidth = 2 * d;
    ctx.strokeStyle = 'rgba(255,255,255,0.95)';
    ctx.beginPath();
    ctx.arc(x, y, 9 * d, 0, Math.PI * 2);
    ctx.stroke();
    ctx.strokeStyle = 'rgba(232,90,44,0.95)';
    ctx.lineWidth = 1.5 * d;
    ctx.beginPath();
    ctx.arc(x, y, 6.5 * d, 0, Math.PI * 2);
    ctx.stroke();
    ctx.restore();
  }

  function setView(next: View, nextStyle: ViewStyle) {
    view = next;
    setStyle(nextStyle);
    setZoomPct(zoomPercent(view, dpr()));
    paint();
    // The zoom changed: let the host know how many pixels are worth sending.
    clearTimeout(viewportTimer);
    viewportTimer = setTimeout(sendViewport, 300);
  }

  function sendViewport() {
    if (live() && wrap) void session?.send({ op: 'viewport', viewport: viewportForView(boxSize(), dpr(), view, screen) });
  }

  function applyStyle(next: ViewStyle) {
    if (!screen.width) return;
    const box = boxSize();
    if (next === 'original') {
      const focus = padMode() ? { x: cursor().x * screen.width, y: cursor().y * screen.height } : undefined;
      setView(oneToOneView(box, screen, dpr(), focus), 'original');
    } else if (next === 'adaptive') {
      setView(fitView(box, screen), 'adaptive');
    }
  }

  function refit() {
    if (!screen.width) return;
    const box = boxSize();
    if (style() === 'adaptive') view = fitView(box, screen);
    else if (style() === 'original') view = clampView({ ...view, scale: originalScale(dpr()) }, box, screen);
    else view = clampView(view, box, screen);
    setZoomPct(zoomPercent(view, dpr()));
    paint();
  }

  function ensureDecoder(codec: string): DecoderLike | undefined {
    const VD = (globalThis as { VideoDecoder?: new (init: unknown) => DecoderLike }).VideoDecoder;
    if (!VD) {
      session?.fail({ kind: 'no-webcodecs', message: 'This web view has no WebCodecs video decoder.' });
      return undefined;
    }
    if (decoder && decoder.state !== 'closed' && decoderCodec === codec) return decoder;
    try {
      decoder?.close();
    } catch {
      /* closed */
    }
    decoderCodec = codec;
    decoder = new VD({
      output: (frame: VideoFrame) => {
        if (lastBitmap && 'close' in lastBitmap) lastBitmap.close();
        lastBitmap = frame;
        picture = { width: frame.displayWidth, height: frame.displayHeight };
        if (picSize().width !== picture.width || picSize().height !== picture.height) setPicSize({ ...picture });
        const first = !hasFrame();
        setHasFrame(true);
        if (first) {
          refit();
          if (style() === 'original') applyStyle('original');
        }
        paint();
      },
      error: (e: Error) => {
        // A broken stream: ask for a key frame on a fresh decoder.
        decoder = undefined;
        if (!session || session.isEnded) return;
        if (decodeErrors++ >= 3) {
          session.fail({ kind: 'decoder', message: e?.message || 'The video decoder failed.' });
          return;
        }
        void session.send({ op: 'keyframe' });
      },
    });
    // Configured on the first key frame (decode): the AVC description comes from its SPS/PPS.
    description = undefined;
    needKey = true;
    return decoder;
  }
  let needKey = true;
  let decodeErrors = 0;
  let description: Uint8Array | undefined;

  function decode(frame: Frame) {
    const d = decoder && decoder.state !== 'closed' ? decoder : decoderCodec ? ensureDecoder(decoderCodec) : undefined;
    if (!d) return;
    if (needKey && !frame.key) return;
    const Chunk = (globalThis as { EncodedVideoChunk?: new (init: unknown) => unknown }).EncodedVideoChunk;
    if (!Chunk) return;
    try {
      // AVC (length-prefixed + avcC), not Annex-B: WebKitGTK only decodes AVC (lib/screen.ts).
      if (frame.key) {
        const next = avcDescription(frame.data);
        if (next && (d.state !== 'configured' || !sameBytes(next, description))) {
          description = next;
          d.configure({ codec: decoderCodec, description: next, optimizeForLatency: true, hardwareAcceleration: 'no-preference' });
        }
      }
      if (d.state !== 'configured') return;
      needKey = false;
      d.decode(new Chunk({ type: frame.key ? 'key' : 'delta', timestamp: frame.tsMs * 1000, data: annexBToAvc(frame.data) }));
    } catch {
      needKey = true;
      void session?.send({ op: 'keyframe' });
    }
  }

  // ------------------------------------------------------------ session
  function start() {
    session?.stop();
    clearTimeout(reconnectTimer);
    setHasFrame(false);
    decodeErrors = 0;
    const box = wrap ? boxSize() : { width: 412, height: 780 };
    const s: ScreenSession = new ScreenSession(props.hostId, {
      onState: (next) => {
        if (s !== session) return;
        if (next.kind === 'live') {
          screen = { width: next.width, height: next.height };
          if (next.display) setDisplay(next.display);
          refit();
        }
        setState(next);
      },
      onConfig: (codec) => {
        if (s === session) ensureDecoder(codec);
      },
      onStats: (st) => {
        if (s === session) setStats({ fps: st.fps, kbps: st.kbps });
      },
      onFrame: (frame) => {
        if (s === session) decode(frame);
      },
    });
    session = s;
    void s.open({ viewport: viewportFor(box, dpr()), quality: quality(), viewOnly: props.viewOnly, display: display() });
  }

  onMount(() => {
    void webCodecsSupport().then((ok) => {
      if (!ok) {
        setState({ kind: 'problem', problem: { kind: 'no-webcodecs', message: 'This web view has no H.264 WebCodecs decoder.' } });
        return;
      }
      start();
    });
    let timer: ReturnType<typeof setTimeout> | undefined;
    const observer = new ResizeObserver(() => {
      refit();
      clearTimeout(timer);
      timer = setTimeout(sendViewport, 250);
    });
    observer.observe(wrap);
    const onFs = () => setFullscreen(!!document.fullscreenElement);
    document.addEventListener('fullscreenchange', onFs);
    onCleanup(() => {
      observer.disconnect();
      clearTimeout(timer);
      document.removeEventListener('fullscreenchange', onFs);
    });
  });
  onCleanup(() => {
    releaseKeys();
    session?.stop();
    clearTimeout(reconnectTimer);
    clearTimeout(longTimer);
    clearTimeout(hideTimer);
    clearTimeout(toastTimer);
    clearTimeout(viewportTimer);
    clearInterval(edgeTimer);
    if (document.fullscreenElement) void document.exitFullscreen?.().catch(() => {});
    try {
      decoder?.close();
    } catch {
      /* closed */
    }
    if (lastBitmap && 'close' in lastBitmap) lastBitmap.close();
  });

  // Auto-reconnect once after a dropped connection (not after a refusal).
  createEffect(() => {
    const p = problem();
    if (p?.kind === 'lost' && attempt() === 0) {
      reconnectTimer = setTimeout(() => {
        setAttempt(1);
        start();
      }, 1500);
    }
  });

  const send = (cmd: Parameters<ScreenSession['send']>[0]) => {
    if (viewOnly()) return;
    held.track(cmd);
    void session?.send(cmd);
  };
  function releaseKeys() {
    for (const c of held.releaseAll()) void session?.send(c);
  }

  // ------------------------------------------------------------ pointer
  const local = (e: { clientX: number; clientY: number }) => {
    const r = wrap.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  const screenPoint = (p: { x: number; y: number }) => toScreen(p.x, p.y, view, screen);

  function pointerTo(p: { x: number; y: number }) {
    setCursor(p);
    pad.setCursor(p.x, p.y);
    send({ op: 'pointer', ...p });
  }

  function apply(actions: GestureAction[]) {
    for (const a of actions) {
      switch (a.kind) {
        case 'move':
          pointerTo(screenPoint(a));
          break;
        case 'button':
          send({ op: 'button', button: a.button, down: a.down });
          if (!a.down && a.button === 'right') navigator.vibrate?.(10);
          break;
        case 'scroll':
          send({ op: 'scroll', dx: a.dx, dy: a.dy });
          break;
        case 'zoom':
          setView(zoomAt(view, a.factor, a.cx, a.cy, boxSize(), screen), 'custom');
          break;
        case 'pan':
          setView(clampView({ ...view, x: view.x + a.dx, y: view.y + a.dy }, boxSize(), screen), style() === 'adaptive' && isFit(view, boxSize(), screen) ? 'adaptive' : style());
          break;
        case 'long-press-armed':
          clearTimeout(longTimer);
          longTimer = setTimeout(() => apply(gestures.longPress()), LONG_PRESS_MS);
          break;
      }
    }
  }

  function applyPad(actions: PadAction[]) {
    for (const a of actions) {
      switch (a.kind) {
        case 'cursor':
          setCursor({ x: a.x, y: a.y });
          send({ op: 'pointer', x: a.x, y: a.y });
          // Keep the cursor on screen when zoomed in.
          if (!isFit(view, boxSize(), screen)) {
            view = followCursor(view, boxSize(), screen, a);
          }
          paint();
          break;
        case 'button':
          send({ op: 'button', button: a.button, down: a.down });
          if (!a.down && a.button === 'right') navigator.vibrate?.(10);
          break;
        case 'scroll':
          send({ op: 'scroll', dx: a.dx, dy: a.dy });
          break;
        case 'zoom':
          setView(zoomAt(view, a.factor, a.cx, a.cy, boxSize(), screen), 'custom');
          break;
        case 'pan':
          setView(clampView({ ...view, x: view.x + a.dx, y: view.y + a.dy }, boxSize(), screen), style());
          break;
        case 'long-press-armed':
          clearTimeout(longTimer);
          longTimer = setTimeout(() => applyPad(pad.longPress()), LONG_PRESS_MS);
          break;
      }
    }
  }

  const mouseButton = (b: number) => (b === 2 ? 'right' : b === 1 ? 'middle' : 'left');

  function onPointerDown(e: PointerEvent) {
    setMenu(undefined);
    if (!live() || !screen.width) return;
    canvas.focus({ preventScroll: true });
    canvas.setPointerCapture?.(e.pointerId);
    if (e.pointerType === 'mouse') {
      if (e.button === 1) e.preventDefault();
      pointerTo(screenPoint(local(e)));
      send({ op: 'button', button: mouseButton(e.button), down: true });
      return;
    }
    if (padMode()) applyPad(pad.down(e.pointerId, local(e), e.timeStamp));
    else apply(gestures.down(e.pointerId, local(e), e.timeStamp));
  }
  function onPointerMove(e: PointerEvent) {
    if (!live() || !screen.width) return;
    if (e.pointerType === 'mouse') {
      const p = local(e);
      lastMouse = p;
      if (!viewOnly()) pointerTo(screenPoint(p));
      // Top edge brings the toolbar back (desktop).
      if (p.y < 6 && !barShown()) showBar();
      return;
    }
    if (padMode()) {
      const actions = pad.move(e.pointerId, local(e));
      if (actions.length) clearTimeout(longTimer);
      applyPad(actions);
      return;
    }
    const actions = gestures.move(e.pointerId, local(e));
    if (actions.length) clearTimeout(longTimer);
    apply(actions);
  }
  function onPointerUp(e: PointerEvent) {
    clearTimeout(longTimer);
    if (!live()) return;
    if (e.pointerType === 'mouse') {
      send({ op: 'button', button: mouseButton(e.button), down: false });
      return;
    }
    if (padMode()) applyPad(e.type === 'pointercancel' ? pad.cancel() : pad.up(e.pointerId, local(e), e.timeStamp));
    else apply(e.type === 'pointercancel' ? gestures.cancel() : gestures.up(e.pointerId, local(e), e.timeStamp));
  }
  function onWheel(e: WheelEvent) {
    if (!live()) return;
    e.preventDefault();
    if (e.ctrlKey) {
      // Trackpad pinch / Ctrl+wheel: zoom the picture locally.
      const p = local(e);
      setView(zoomAt(view, Math.exp(-e.deltaY / 200), p.x, p.y, boxSize(), screen), 'custom');
      return;
    }
    const unit = e.deltaMode === 1 ? 1 / 3 : e.deltaMode === 2 ? 5 : 1 / 100;
    send({ op: 'scroll', dx: e.deltaX * unit, dy: e.deltaY * unit });
  }
  // Desktop "Original" view bigger than the window: pan at the edges.
  createEffect(() => {
    clearInterval(edgeTimer);
    if (phone() || !live()) return;
    edgeTimer = setInterval(() => {
      if (!lastMouse || !screen.width || isFit(view, boxSize(), screen)) return;
      const box = boxSize();
      const d = edgePan(lastMouse, box);
      if (!d.dx && !d.dy) return;
      const next = clampView({ ...view, x: view.x + d.dx, y: view.y + d.dy }, box, screen);
      if (next.x !== view.x || next.y !== view.y) {
        view = next;
        paint();
      }
    }, 33);
  });

  // ------------------------------------------------------------ keyboard
  function onCanvasKey(e: KeyboardEvent, down: boolean) {
    if (!live()) return;
    if (down && isEscapeHatch(e)) {
      e.preventDefault();
      releaseKeys();
      canvas.blur();
      setCaptured(false);
      showBar();
      flash('Keyboard released — click the picture to send keys again');
      return;
    }
    const cmd = hardwareKey(e, down);
    if (!cmd) return;
    e.preventDefault();
    e.stopPropagation();
    send(cmd);
  }
  function onImeKey(e: KeyboardEvent) {
    const special = ['Enter', 'Tab', 'Escape', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'];
    if (special.includes(e.key) || (e.key === 'Backspace' && ime.value === '')) {
      e.preventDefault();
      const k = keysymFor(e.key);
      if (k !== undefined) for (const c of pressCommands(k, mods())) send(c);
      clearMods();
    }
  }
  function onImeInput() {
    const diff = diffImeValue(ime.value, imeState);
    imeState = diff.state;
    if (diff.resetValue !== undefined) ime.value = diff.resetValue;
    for (const c of imeToCommands(diff.bytes, mods())) send(c);
    if (diff.bytes) clearMods();
  }
  function clearMods() {
    if (mods().length) setMods([]);
  }
  function toggleMod(m: Modifier) {
    setMods((list) => (list.includes(m) ? list.filter((x) => x !== m) : [...list, m]));
  }
  function pressKey(keysym: number) {
    for (const c of pressCommands(keysym, mods())) send(c);
    clearMods();
  }
  function sendChord(k: SpecialKey) {
    for (const c of chordCommands(k.keysyms)) send(c);
    setMenu(undefined);
    flash(`Sent ${k.label}`);
  }
  function toggleKeyboard() {
    setMenu(undefined);
    if (keyboard()) {
      ime.blur();
      setKeyboard(false);
      return;
    }
    ime.value = '';
    imeState = imeInitial();
    setKeyboard(true);
    if (phone()) ime.focus({ preventScroll: true });
    else canvas.focus({ preventScroll: true });
  }

  // ------------------------------------------------------------ actions
  const pickQuality = (q: DesktopQuality) => {
    setQuality(q);
    setMenu(undefined);
    void session?.send({ op: 'quality', quality: q });
  };
  const pickStyle = (next: 'adaptive' | 'original') => {
    setMenu(undefined);
    applyStyle(next);
  };
  const pickDisplay = (id: string) => {
    setMenu(undefined);
    if (id === display()) return;
    setDisplay(id);
    setAttempt(0);
    start();
  };
  const toggleFullscreen = async () => {
    setMenu(undefined);
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await wrap.parentElement?.requestFullscreen?.();
    } catch {
      flash("Fullscreen isn't available here");
    }
  };
  const retry = () => {
    setAttempt(0);
    start();
  };

  // Mouse-mode on-screen buttons (phone): hold = drag.
  const padButton = (button: 'left' | 'right', down: boolean) => {
    send({ op: 'pointer', ...cursor() });
    send({ op: 'button', button, down });
    if (down && button === 'right') navigator.vibrate?.(10);
  };

  const statusLine = () => {
    const s = state();
    if (s.kind === 'connecting') return `Connecting to ${props.hostName}…`;
    if (s.kind === 'consent') return 'Waiting for the PC';
    if (s.kind === 'live') {
      const size = picSize().width ? `${picSize().width}×${picSize().height}` : `${s.width}×${s.height}`;
      return stats() ? `${size} · ${Math.round(stats()!.fps)} fps · ${(stats()!.kbps / 1000).toFixed(1)} Mbps` : size;
    }
    return '';
  };
  const displayName = (id?: string) => displays().find((d) => d.id === id)?.name ?? id;

  const ToolButton = (p: { label: string; testId?: string; pressed?: boolean; expanded?: boolean; disabled?: boolean; onClick: () => void; children: JSX.Element }) => (
    <button
      type="button"
      class="ghost-icon-button desktop-tool"
      aria-label={p.label}
      title={p.label}
      data-testid={p.testId}
      aria-pressed={p.pressed}
      aria-expanded={p.expanded}
      aria-haspopup={p.expanded !== undefined ? 'menu' : undefined}
      disabled={p.disabled}
      onClick={p.onClick}
    >
      {p.children}
    </button>
  );

  return (
    <div
      class="preview-viewer desktop-viewer"
      data-testid="desktop-viewer"
      data-state={state().kind}
      data-bar={barShown() ? 'shown' : 'hidden'}
      data-fullscreen={fullscreen() ? 'true' : undefined}
    >
      <header
        class="desktop-bar"
        classList={{ 'desktop-bar-hidden': !barShown() }}
        data-testid="desktop-toolbar"
        onPointerEnter={() => clearTimeout(hideTimer)}
        onPointerLeave={scheduleHide}
      >
        <button type="button" class="ghost-icon-button desktop-tool" aria-label="Close desktop" title="Close" onClick={props.onClose}>
          <X size={19} />
        </button>
        <div class="desktop-title">
          <span class="desktop-title-name">
            <Monitor size={14} /> {props.hostName}
            <Show when={canPickDisplay() && display()}>
              <span class="desktop-title-display">· {displayName(display())}</span>
            </Show>
          </span>
          <span class="desktop-title-meta" data-testid="desktop-status-line">
            {statusLine()}
          </span>
        </div>
        <Show when={viewOnly()}>
          <span class="desktop-badge" data-testid="desktop-view-only" title="Taps and keys are not sent to the PC">
            <Eye size={13} /> View only
          </span>
        </Show>
        <div class="desktop-tools" role="toolbar" aria-label="Remote desktop">
          <Show when={canPickDisplay()}>
            <div class="desktop-quality">
              <ToolButton label="Display" testId="desktop-display" expanded={menu() === 'display'} disabled={!live()} onClick={() => toggleMenu('display')}>
                <MonitorCog size={18} />
              </ToolButton>
              <Show when={menu() === 'display'}>
                <div class="desktop-menu" role="menu" data-testid="desktop-display-menu">
                  <p class="desktop-menu-title">Display</p>
                  <For each={displays()}>
                    {(d) => (
                      <button type="button" role="menuitemradio" aria-checked={(display() ?? displays().find((x) => x.primary)?.id) === d.id} class="desktop-menu-item" onClick={() => pickDisplay(d.id)}>
                        <span class="desktop-menu-label">{d.name}</span>
                        <span class="desktop-menu-hint">
                          {d.width}×{d.height}
                          {d.primary ? ' · primary' : ''} · {d.id}
                        </span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Show>
          <div class="desktop-quality">
            <ToolButton label={`View: ${style() === 'original' ? 'Original' : style() === 'adaptive' ? 'Scale adaptive' : `${zoomPct()}%`}`} testId="desktop-zoom" expanded={menu() === 'view'} disabled={!live() || !hasFrame()} onClick={() => toggleMenu('view')}>
              <Show when={style() === 'adaptive'} fallback={<span class="desktop-zoom-pct">{style() === 'original' ? '1:1' : `${zoomPct()}%`}</span>}>
                <Expand size={18} />
              </Show>
            </ToolButton>
            <Show when={menu() === 'view'}>
              <div class="desktop-menu" role="menu" data-testid="desktop-view-menu">
                <p class="desktop-menu-title">View style</p>
                <button type="button" role="menuitemradio" aria-checked={style() === 'adaptive'} class="desktop-menu-item" onClick={() => pickStyle('adaptive')}>
                  <span class="desktop-menu-label">Scale adaptive</span>
                  <span class="desktop-menu-hint">Fit the whole screen</span>
                </button>
                <button type="button" role="menuitemradio" aria-checked={style() === 'original'} class="desktop-menu-item" onClick={() => pickStyle('original')}>
                  <span class="desktop-menu-label">Original (1:1)</span>
                  <span class="desktop-menu-hint">One PC pixel per pixel · pan to move</span>
                </button>
                <Show when={style() === 'custom'}>
                  <p class="desktop-menu-note">Zoomed to {zoomPct()}% (pinch or Ctrl + wheel)</p>
                </Show>
                <Show when={picSize().width && picSize().width < screen.width}>
                  <p class="desktop-menu-note" data-testid="desktop-downscaled">
                    Streaming {picSize().width}×{picSize().height} of {screen.width}×{screen.height}. Pick <b>Best quality</b> for every pixel.
                  </p>
                </Show>
              </div>
            </Show>
          </div>
          <div class="desktop-quality">
            <ToolButton label={`Quality: ${qualityLabel(quality())}`} testId="desktop-quality" expanded={menu() === 'quality'} disabled={!live()} onClick={() => toggleMenu('quality')}>
              <Gauge size={18} />
            </ToolButton>
            <Show when={menu() === 'quality'}>
              <div class="desktop-menu" role="menu" data-testid="desktop-quality-menu">
                <p class="desktop-menu-title">Image quality</p>
                <For each={options()}>
                  {(q) => (
                    <button type="button" role="menuitemradio" aria-checked={quality() === q.value} class="desktop-menu-item" onClick={() => pickQuality(q.value)}>
                      <span class="desktop-menu-label">{q.label}</span>
                      <span class="desktop-menu-hint">{q.hint}</span>
                    </button>
                  )}
                </For>
                <Show when={!hasFeature(props.status, 'presets')}>
                  <p class="desktop-menu-note">Update unipi-host on {props.hostName} for Best quality.</p>
                </Show>
              </div>
            </Show>
          </div>
          <Show when={!viewOnly()}>
            <div class="desktop-quality">
              <ToolButton label="Special keys" testId="desktop-special" expanded={menu() === 'keys'} disabled={!live()} onClick={() => toggleMenu('keys')}>
                <Command size={18} />
              </ToolButton>
              <Show when={menu() === 'keys'}>
                <div class="desktop-menu" role="menu" data-testid="desktop-keys-menu">
                  <p class="desktop-menu-title">Send to the PC</p>
                  <For each={SPECIAL_KEYS}>
                    {(k) => (
                      <button type="button" role="menuitem" class="desktop-menu-item desktop-menu-row" onClick={() => sendChord(k)}>
                        <span class="desktop-menu-label">{k.label}</span>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>
            <ToolButton label={keyboard() ? 'Hide keyboard' : 'Keyboard'} pressed={keyboard()} disabled={!live()} onClick={toggleKeyboard}>
              <Keyboard size={18} />
            </ToolButton>
            <Show when={phone()}>
              <ToolButton
                label={mouseMode() === 'mouse' ? 'Mouse mode (tap for touch mode)' : 'Touch mode (tap for mouse mode)'}
                testId="desktop-mouse-mode"
                pressed={mouseMode() === 'mouse'}
                disabled={!live()}
                onClick={() => {
                  const next = mouseMode() === 'mouse' ? 'touch' : 'mouse';
                  setMouseMode(next);
                  flash(next === 'mouse' ? 'Mouse mode: drag to move the pointer, tap to click' : 'Touch mode: tap where you want to click');
                  paint();
                }}
              >
                <Show when={mouseMode() === 'mouse'} fallback={<Hand size={18} />}>
                  <MousePointer2 size={18} />
                </Show>
              </ToolButton>
            </Show>
          </Show>
          <ToolButton label={fullscreen() ? 'Exit fullscreen' : 'Fullscreen'} testId="desktop-fullscreen" pressed={fullscreen()} onClick={() => void toggleFullscreen()}>
            <Show when={fullscreen()} fallback={<Maximize size={17} />}>
              <Minimize size={17} />
            </Show>
          </ToolButton>
          <ToolButton label={pinned() ? 'Unpin toolbar' : 'Pin toolbar'} testId="desktop-pin" pressed={pinned()} onClick={() => setPinned((p) => !p)}>
            <Show when={pinned()} fallback={<Pin size={16} />}>
              <PinOff size={16} />
            </Show>
          </ToolButton>
        </div>
      </header>
      <Show when={!barShown()}>
        <button type="button" class="desktop-bar-handle" aria-label="Show toolbar" data-testid="desktop-bar-handle" onClick={showBar}>
          <ChevronDown size={16} />
        </button>
      </Show>
      <div class="desktop-canvas-wrap browser-canvas-wrap" ref={wrap} onClick={() => setMenu(undefined)}>
        <canvas
          ref={canvas}
          class="browser-canvas desktop-canvas"
          data-cursor={viewOnly() ? 'view' : 'control'}
          data-mode={padMode() ? 'mouse' : 'touch'}
          tabindex="0"
          aria-label={`Screen of ${props.hostName}`}
          data-testid="desktop-canvas"
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerUp}
          onPointerLeave={() => (lastMouse = undefined)}
          onWheel={onWheel}
          onKeyDown={(e) => onCanvasKey(e, true)}
          onKeyUp={(e) => onCanvasKey(e, false)}
          onFocus={() => setCaptured(true)}
          onBlur={() => {
            setCaptured(false);
            releaseKeys();
          }}
          onContextMenu={(e) => e.preventDefault()}
          onAuxClick={(e) => e.preventDefault()}
        />
        <textarea
          ref={ime}
          class="browser-ime"
          aria-label="Type on the PC"
          autocapitalize="off"
          autocomplete="off"
          spellcheck={false}
          onInput={onImeInput}
          onKeyDown={onImeKey}
          onBlur={() => phone() && setKeyboard(false)}
        />
        <Show when={state().kind === 'connecting' || (live() && !hasFrame())}>
          <div class="browser-overlay" data-testid="desktop-connecting">
            <RefreshCw size={22} class="refresh-spinning" />
            <p>{state().kind === 'connecting' ? (attempt() ? `Reconnecting to ${props.hostName}…` : `Connecting to ${props.hostName}…`) : 'Waiting for the first picture…'}</p>
          </div>
        </Show>
        <Show when={state().kind === 'consent'}>
          <div class="browser-overlay desktop-consent" data-testid="desktop-consent" role="status">
            <span class="browser-problem-icon">
              <ShieldQuestion size={24} />
            </span>
            <h2>Allow sharing on {props.hostName}</h2>
            <p>
              The PC is showing a "Share your screen" dialog. Someone at the PC has to choose the screen and press{' '}
              <b>Share</b>. It's remembered for next time.
            </p>
            <RefreshCw size={18} class="refresh-spinning" />
          </div>
        </Show>
        <Show when={problem()}>
          {(p) => (
            <div class="browser-overlay browser-problem" data-kind={p().kind} role="alert" data-testid="desktop-problem">
              <span class="browser-problem-icon">
                <Show when={p().kind === 'disabled' || p().kind === 'lost'} fallback={<TriangleAlert size={22} />}>
                  <Unplug size={22} />
                </Show>
              </span>
              <h2>{problemTitle(p().kind)}</h2>
              <Show when={p().message !== problemTitle(p().kind)}>
                <p>{p().message}</p>
              </Show>
              <Show when={p().kind === 'disabled'}>
                <props.CommandBox command={ENABLE_DESKTOP_COMMAND} />
              </Show>
              <Show when={p().hint && p().kind !== 'disabled'}>
                <p class="preview-muted">{p().hint}</p>
              </Show>
              <Show when={p().kind === 'no-webcodecs'}>
                <p class="preview-muted">Update Android System WebView (or Chrome) from the Play Store, then try again.</p>
              </Show>
              <div class="browser-problem-actions">
                <Show when={p().kind !== 'no-webcodecs' && p().kind !== 'not-compiled'}>
                  <button type="button" class="preview-btn primary" onClick={retry}>
                    <RefreshCw size={16} /> {p().kind === 'replaced' ? 'Take over' : p().kind === 'lost' ? 'Reconnect' : 'Try again'}
                  </button>
                </Show>
                <button type="button" class="preview-btn" onClick={props.onClose}>
                  Back
                </button>
              </div>
            </div>
          )}
        </Show>
        <Show when={live() && hasFrame() && phone() && !viewOnly() && style() === 'adaptive' && !keyboard()}>
          <p class="desktop-hint" data-testid="desktop-gesture-hint">
            <Show
              when={padMode()}
              fallback={
                <>
                  <Hand size={12} /> Tap click · hold right-click · 2 fingers scroll · pinch zoom
                </>
              }
            >
              <MousePointer2 size={12} /> Drag moves the pointer · tap click · tap-tap-drag selects
            </Show>
          </p>
        </Show>
        <Show when={live() && hasFrame() && !phone() && !viewOnly()}>
          <p class="desktop-capture-hint" data-testid="desktop-capture-hint" data-captured={captured() ? 'true' : 'false'}>
            {captured() ? 'Keys go to the PC · Ctrl+Shift+F12 releases' : 'Click the picture to control the PC'}
          </p>
        </Show>
        <Show when={toast()}>
          <p class="desktop-toast" role="status" data-testid="desktop-toast">
            {toast()}
          </p>
        </Show>
        <Show when={live() && hasFrame() && phone() && !viewOnly()}>
          <div class="desktop-fabs" data-testid="desktop-fabs">
            <Show when={padMode()}>
              <button
                type="button"
                class="desktop-fab desktop-fab-wide"
                aria-label="Left click (hold to drag)"
                onPointerDown={(e) => {
                  e.stopPropagation();
                  padButton('left', true);
                }}
                onPointerUp={(e) => {
                  e.stopPropagation();
                  padButton('left', false);
                }}
                onPointerCancel={() => padButton('left', false)}
              >
                L
              </button>
              <button
                type="button"
                class="desktop-fab desktop-fab-wide"
                aria-label="Right click"
                onPointerDown={(e) => {
                  e.stopPropagation();
                  padButton('right', true);
                }}
                onPointerUp={(e) => {
                  e.stopPropagation();
                  padButton('right', false);
                }}
                onPointerCancel={() => padButton('right', false)}
              >
                R
              </button>
            </Show>
            <button type="button" class="desktop-fab" aria-label={keyboard() ? 'Hide keyboard' : 'Show keyboard'} aria-pressed={keyboard()} onClick={toggleKeyboard}>
              <Keyboard size={18} />
            </button>
          </div>
        </Show>
      </div>
      <Show when={keyboard() && live() && !viewOnly()}>
        <div class="term-toolbar desktop-keys" role="toolbar" aria-label="Keys for the PC" data-testid="desktop-keys">
          <div class="term-key-scroll">
            <For each={KEY_ROW}>
              {(k) => (
                <button
                  type="button"
                  class={`term-key ${k.icon ? '' : 'term-key-text'} ${k.mod && mods().includes(k.mod) ? 'term-key-lit' : ''}`}
                  aria-label={k.aria}
                  aria-pressed={k.mod ? mods().includes(k.mod) : undefined}
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => (k.mod ? toggleMod(k.mod) : k.keysym !== undefined && pressKey(k.keysym))}
                >
                  <Show when={k.icon} fallback={<span class="term-key-label">{k.label}</span>}>
                    {k.icon!()}
                  </Show>
                </button>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
}
