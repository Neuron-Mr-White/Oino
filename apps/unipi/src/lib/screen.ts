/**
 * Desktop streaming (UNI-216, forwarding phase 4): the UI's seam onto the
 * core's `screen_status` / `screen_open` / `screen_cmd` / `screen_stop`
 * (`src-tauri/src/screen.rs`) — the PC's whole screen as H.264, decoded
 * with WebCodecs onto a canvas, driven with taps/drags/keys.
 *
 * Pure helpers carry the rules so they are unit-tested: `statusView`,
 * `problemFrom*`, `parseFrame`, `keysymFor`/`keyCommands`, `textCommands`,
 * the view transform (`fitView`, `zoomAt`, `clampView`, `toScreen`) and the
 * touch gesture machine (`GestureMachine`). `ScreenSession` owns one stream.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { z } from 'zod';
import {
  DesktopEventSchema,
  DesktopStatusDataSchema,
  toAppError,
  type AppError,
  type DesktopCmd,
  type DesktopEvent,
  type DesktopQuality,
  type DesktopStatusData,
  type DesktopViewport,
} from '@unipi/protocol';
import { call, type Result } from './ipc';
import { can } from './platform';

/** Desktop streaming needs the native core (iroh streams). */
export function canStream(): boolean {
  return can('desktopStream');
}

export const ENABLE_DESKTOP_COMMAND = 'unipi-host desktop enable';
export const ENABLE_DESKTOP_VIEW_COMMAND = 'unipi-host desktop enable --view-only';

/** Does this WebView have WebCodecs H.264? */
export async function webCodecsSupport(codec = 'avc1.42e01f'): Promise<boolean> {
  const VD = (globalThis as { VideoDecoder?: { isConfigSupported(c: unknown): Promise<{ supported?: boolean }> } }).VideoDecoder;
  if (!VD) return false;
  try {
    return !!(await VD.isConfigSupported({ codec, optimizeForLatency: true })).supported;
  } catch {
    return false;
  }
}

// ------------------------------------------------------------- status

export function screenStatus(hostId: string): Promise<Result<DesktopStatusData>> {
  return transport().status(hostId);
}

export type StatusView =
  | { kind: 'loading' }
  | { kind: 'disabled' }
  | { kind: 'unavailable'; reason: string; hint?: string; status?: DesktopStatusData }
  | { kind: 'error'; error: AppError }
  | { kind: 'ready'; status: DesktopStatusData };

export function statusView(result: Result<DesktopStatusData> | undefined): StatusView {
  if (!result) return { kind: 'loading' };
  if (!result.ok) {
    if (result.error.code === 'E_DESKTOP_DISABLED') return { kind: 'disabled' };
    if (result.error.code === 'E_UNSUPPORTED' || result.error.code === 'E_SCHEMA') {
      return { kind: 'error', error: { ...result.error, message: 'This PC runs an older unipi-host without desktop streaming.', hint: 'Update unipi-host on the PC.' } };
    }
    return { kind: 'error', error: result.error };
  }
  const status = result.value;
  if (!status.enabled) return { kind: 'disabled' };
  if (!status.backend) return { kind: 'unavailable', reason: status.reason ?? 'no_display', hint: status.hint, status };
  return { kind: 'ready', status };
}

/** "Wayland" / "X11" / "macOS" / "Windows" for the ready card (UNI-252: GNOME's
 *  own screen-cast API reads as "GNOME"). */
export function displayLabel(status: Pick<DesktopStatusData, 'display' | 'backend'>): string {
  if (status.backend === 'mutter') return 'GNOME';
  switch (status.display) {
    case 'wayland':
      return 'Wayland';
    case 'x11':
      return 'X11';
    case 'macos':
      return 'macOS';
    case 'windows':
      return 'Windows';
    default:
      return 'Desktop';
  }
}

/** A session-level problem → what the viewer overlay shows. */
export type ProblemKind =
  | 'disabled'
  | 'denied'
  | 'no-display'
  | 'not-compiled'
  | 'encoder'
  | 'decoder'
  | 'no-webcodecs'
  | 'replaced'
  | 'ended'
  | 'lost';

export type Problem = { kind: ProblemKind; message: string; hint?: string };

export function problemFromError(error: { code?: string; message: string; hint?: string; reason?: string }): Problem {
  const { code, message, hint, reason } = error;
  if (code === 'E_DESKTOP_DISABLED') return { kind: 'disabled', message, hint };
  if (code === 'E_DESKTOP_UNAVAILABLE') {
    switch (reason) {
      case 'denied':
        return { kind: 'denied', message, hint };
      case 'no_display':
      case 'no_portal':
        return { kind: 'no-display', message, hint };
      case 'not_compiled':
      case 'no_gstreamer':
        return { kind: 'not-compiled', message, hint };
      case 'encoder_failed':
        return { kind: 'encoder', message, hint };
      default:
        return { kind: 'lost', message, hint };
    }
  }
  return { kind: 'lost', message: message || 'Connection to the PC was lost.', hint };
}

export function problemFromClose(reason: string): Problem {
  if (reason === 'replaced') return { kind: 'replaced', message: 'Another device started watching this PC.' };
  if (reason === 'capture_ended') return { kind: 'ended', message: 'The PC stopped sharing its screen.' };
  if (reason === 'host_exit') return { kind: 'ended', message: 'unipi-host stopped on the PC.' };
  return { kind: 'ended', message: 'The desktop stream ended.' };
}

export function problemTitle(kind: ProblemKind): string {
  switch (kind) {
    case 'disabled':
      return 'Desktop streaming is off on this PC';
    case 'denied':
      return 'Sharing was declined on the PC';
    case 'no-display':
      return 'No desktop to show';
    case 'not-compiled':
      return "This PC's unipi-host can't stream its desktop";
    case 'encoder':
      return 'The video encoder failed on the PC';
    case 'decoder':
      return "This device couldn't decode the video";
    case 'no-webcodecs':
      return "This device can't play the stream";
    case 'replaced':
      return 'Watching from another device';
    case 'ended':
      return 'The stream ended';
    case 'lost':
      return 'Connection lost';
  }
}

// ------------------------------------------------------------- frames

/** One H.264 access unit from the core: 13-byte header + Annex-B. */
export type Frame = { seq: number; key: boolean; tsMs: number; data: Uint8Array };

export function parseFrame(bytes: Uint8Array): Frame | undefined {
  if (bytes.length < 17) return undefined;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const data = bytes.subarray(13);
  if (!(data[0] === 0 && data[1] === 0 && (data[2] === 1 || (data[2] === 0 && data[3] === 1)))) return undefined;
  return { seq: view.getUint32(0), key: bytes[4] === 1, tsMs: Number(view.getBigUint64(5)), data };
}

export function encodeFrame(seq: number, key: boolean, tsMs: number, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(13 + data.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, seq);
  out[4] = key ? 1 : 0;
  view.setBigUint64(5, BigInt(tsMs));
  out.set(data, 13);
  return out;
}

// ------------------------------------------------------------- view transform

/**
 * How the screen is shown in the canvas box: `scale` = screen px → CSS px,
 * `x`/`y` = where the screen's top-left lands (CSS px, may be negative when
 * zoomed in and panned).
 */
export type View = { scale: number; x: number; y: number };
export type Size = { width: number; height: number };

/** Fit (contain, centred) — the default and the "Fit" button. */
export function fitView(box: Size, screen: Size): View {
  if (screen.width <= 0 || screen.height <= 0 || box.width <= 0 || box.height <= 0) return { scale: 1, x: 0, y: 0 };
  const scale = Math.min(box.width / screen.width, box.height / screen.height);
  return { scale, x: (box.width - screen.width * scale) / 2, y: (box.height - screen.height * scale) / 2 };
}

/** 1:1 — one screen pixel per device pixel, centred on `focus` (screen px). */
export function oneToOneView(box: Size, screen: Size, dpr: number, focus?: { x: number; y: number }): View {
  const scale = 1 / (dpr > 0 ? dpr : 1);
  const f = focus ?? { x: screen.width / 2, y: screen.height / 2 };
  return clampView({ scale, x: box.width / 2 - f.x * scale, y: box.height / 2 - f.y * scale }, box, screen);
}

/** Keeps the screen covering the box where it can (centres a smaller axis). */
export function clampView(view: View, box: Size, screen: Size): View {
  const w = screen.width * view.scale;
  const h = screen.height * view.scale;
  const axis = (pos: number, size: number, room: number) => (size <= room ? (room - size) / 2 : Math.min(0, Math.max(room - size, pos)));
  return { scale: view.scale, x: axis(view.x, w, box.width), y: axis(view.y, h, box.height) };
}

/** Zoom by `factor` keeping the point `(cx, cy)` (CSS px in the box) still. */
export function zoomAt(view: View, factor: number, cx: number, cy: number, box: Size, screen: Size): View {
  const min = fitView(box, screen).scale;
  const max = Math.max(min * 8, 4);
  const scale = Math.min(max, Math.max(min, view.scale * factor));
  const k = scale / view.scale;
  return clampView({ scale, x: cx - (cx - view.x) * k, y: cy - (cy - view.y) * k }, box, screen);
}

/** A point in the box → normalised screen coordinates (0..1), clamped. */
export function toScreen(px: number, py: number, view: View, screen: Size): { x: number; y: number } {
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  return { x: clamp((px - view.x) / view.scale / screen.width), y: clamp((py - view.y) / view.scale / screen.height) };
}

/** The viewport hint for the host: the box in device pixels. */
export function viewportFor(box: Size, dpr: number): DesktopViewport {
  const clamp = (v: number) => Math.min(8192, Math.max(16, Math.round(v)));
  const d = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;
  return { width: clamp(box.width * d), height: clamp(box.height * d) };
}

// ------------------------------------------------------------- touch gestures

/** Long-press → right button. */
export const LONG_PRESS_MS = 550;
/** Movement (px) that turns a tap into a drag. */
export const TAP_SLOP = 10;
/** Two-finger scroll: CSS px per wheel step. */
export const SCROLL_PX_PER_STEP = 40;

/**
 * What a touch gesture means for the PC and the local view. The machine is
 * pure: feed it pointer events with timestamps, get actions back.
 *
 * - tap → left click (move + down + up at the tap point)
 * - long-press (no move) → right click
 * - one finger drag (after the slop) → left drag (down at start, moves, up)
 * - two-finger tap → right click
 * - two fingers moving together → scroll (wheel steps)
 * - two fingers pinching → local zoom (and pan with the midpoint)
 */
export type GestureAction =
  | { kind: 'move'; x: number; y: number }
  | { kind: 'button'; button: 'left' | 'right' | 'middle'; down: boolean }
  | { kind: 'scroll'; dx: number; dy: number }
  | { kind: 'zoom'; factor: number; cx: number; cy: number }
  | { kind: 'pan'; dx: number; dy: number }
  | { kind: 'long-press-armed' };

type Pt = { x: number; y: number };

export class GestureMachine {
  private points = new Map<number, Pt>();
  private start: { id: number; at: Pt; time: number } | undefined;
  private dragging = false;
  /** UNI-271: one-finger drag pans the zoomed picture instead of dragging on the PC. */
  private panning: Pt | undefined;
  /** UNI-271: a touch right after a tap: its drag is a left drag even when zoomed. */
  private armDrag = false;
  private lastTap: { at: Pt; time: number } | undefined;
  private longFired = false;
  private two: { startDist: number; lastDist: number; lastMid: Pt; startTime: number; moved: boolean; mode?: 'scroll' | 'pinch'; accX: number; accY: number } | undefined;
  /** True once more than one finger took part (no click on release). */
  private multi = false;

  /** `canPan`: the picture is bigger than the box (zoomed in) — RustDesk touch mode pans then. */
  constructor(private readonly canPan: () => boolean = () => false) {}

  /** The point (box CSS px) where a long-press should click, if armed. */
  longPressPoint(): Pt | undefined {
    return this.start && !this.dragging && !this.panning && !this.multi && !this.longFired ? this.start.at : undefined;
  }

  /** A long-press timer fired. */
  longPress(): GestureAction[] {
    const at = this.longPressPoint();
    if (!at) return [];
    this.longFired = true;
    return [{ kind: 'move', ...at }, { kind: 'button', button: 'right', down: true }, { kind: 'button', button: 'right', down: false }];
  }

  down(id: number, p: Pt, time: number): GestureAction[] {
    this.points.set(id, p);
    if (this.points.size === 1) {
      this.start = { id, at: p, time };
      this.dragging = false;
      this.panning = undefined;
      this.armDrag = !!this.lastTap && time - this.lastTap.time < 300 && Math.hypot(p.x - this.lastTap.at.x, p.y - this.lastTap.at.y) < TAP_SLOP * 4;
      this.longFired = false;
      this.multi = false;
      return [{ kind: 'long-press-armed' }];
    }
    if (this.points.size === 2) {
      const out: GestureAction[] = [];
      if (this.dragging) {
        // A drag in progress ends before the second finger takes over.
        out.push({ kind: 'button', button: 'left', down: false });
        this.dragging = false;
      }
      this.multi = true;
      const [a, b] = [...this.points.values()];
      const dist = Math.hypot(a.x - b.x, a.y - b.y);
      this.two = { startDist: dist, lastDist: dist, lastMid: mid(a, b), startTime: time, moved: false, accX: 0, accY: 0 };
      return out;
    }
    return [];
  }

  move(id: number, p: Pt): GestureAction[] {
    if (!this.points.has(id)) return [];
    this.points.set(id, p);
    if (this.multi && this.points.size < 2) return [];
    if (this.points.size >= 2 && this.two) {
      const [a, b] = [...this.points.values()];
      const dist = Math.hypot(a.x - b.x, a.y - b.y);
      const m = mid(a, b);
      const t = this.two;
      const dMid = { x: m.x - t.lastMid.x, y: m.y - t.lastMid.y };
      if (!t.mode) {
        const spread = Math.abs(dist - t.startDist);
        const travel = Math.hypot(m.x - (t.lastMid.x), m.y - t.lastMid.y) + Math.hypot(t.accX, t.accY);
        if (spread > 24) t.mode = 'pinch';
        else if (travel > TAP_SLOP) t.mode = 'scroll';
        t.accX += dMid.x;
        t.accY += dMid.y;
      }
      const out: GestureAction[] = [];
      if (t.mode === 'pinch') {
        t.moved = true;
        if (t.lastDist > 0) out.push({ kind: 'zoom', factor: dist / t.lastDist, cx: m.x, cy: m.y });
        if (dMid.x || dMid.y) out.push({ kind: 'pan', dx: dMid.x, dy: dMid.y });
      } else if (t.mode === 'scroll') {
        t.moved = true;
        // Content follows the fingers: fingers up → scroll down.
        const dx = -dMid.x / SCROLL_PX_PER_STEP;
        const dy = -dMid.y / SCROLL_PX_PER_STEP;
        if (dx || dy) out.push({ kind: 'scroll', dx, dy });
      }
      t.lastDist = dist;
      t.lastMid = m;
      return out;
    }
    const s = this.start;
    if (!s || s.id !== id || this.multi) return [];
    if (this.panning) {
      const d = { kind: 'pan' as const, dx: p.x - this.panning.x, dy: p.y - this.panning.y };
      this.panning = p;
      return [d];
    }
    if (!this.dragging) {
      if (Math.hypot(p.x - s.at.x, p.y - s.at.y) < TAP_SLOP || this.longFired) return [];
      if (!this.armDrag && this.canPan()) {
        this.panning = p;
        return [{ kind: 'pan', dx: p.x - s.at.x, dy: p.y - s.at.y }];
      }
      this.dragging = true;
      return [{ kind: 'move', ...s.at }, { kind: 'button', button: 'left', down: true }, { kind: 'move', ...p }];
    }
    return [{ kind: 'move', ...p }];
  }

  up(id: number, p: Pt, time: number): GestureAction[] {
    if (!this.points.has(id)) return [];
    this.points.delete(id);
    const out: GestureAction[] = [];
    if (this.multi) {
      if (this.points.size === 0) {
        const t = this.two;
        if (t && !t.moved && time - t.startTime < 400) {
          // Two-finger tap → right click where the first finger was.
          const at = this.start?.at ?? p;
          out.push({ kind: 'move', ...at }, { kind: 'button', button: 'right', down: true }, { kind: 'button', button: 'right', down: false });
        }
        this.reset();
      }
      // Down to one finger: it can't start a new action until lifted (move()
      // ignores single-finger moves while `multi`).
      return out;
    }
    this.lastTap = undefined;
    if (this.dragging) {
      out.push({ kind: 'move', ...p }, { kind: 'button', button: 'left', down: false });
    } else if (this.panning) {
      // A pan clicks nothing.
    } else if (!this.longFired && this.start) {
      out.push({ kind: 'move', ...this.start.at }, { kind: 'button', button: 'left', down: true }, { kind: 'button', button: 'left', down: false });
      if (!this.armDrag) this.lastTap = { at: this.start.at, time };
    }
    this.reset();
    return out;
  }

  cancel(): GestureAction[] {
    const out: GestureAction[] = this.dragging ? [{ kind: 'button', button: 'left', down: false }] : [];
    this.points.clear();
    this.reset();
    return out;
  }

  private reset() {
    this.start = undefined;
    this.dragging = false;
    this.panning = undefined;
    this.armDrag = false;
    this.two = undefined;
    this.multi = false;
  }
}

function mid(a: Pt, b: Pt): Pt {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

// ------------------------------------------------------------- keys

/** X11 keysyms for DOM `key` names that aren't printable characters. */
export const KEYSYMS: Record<string, number> = {
  Enter: 0xff0d,
  Backspace: 0xff08,
  Tab: 0xff09,
  Escape: 0xff1b,
  Delete: 0xffff,
  Insert: 0xff63,
  Home: 0xff50,
  End: 0xff57,
  PageUp: 0xff55,
  PageDown: 0xff56,
  ArrowLeft: 0xff51,
  ArrowUp: 0xff52,
  ArrowRight: 0xff53,
  ArrowDown: 0xff54,
  Shift: 0xffe1,
  Control: 0xffe3,
  Alt: 0xffe9,
  Meta: 0xffeb,
  Super: 0xffeb,
  CapsLock: 0xffe5,
  ContextMenu: 0xff67,
  PrintScreen: 0xff61,
  ' ': 0x20,
  F1: 0xffbe,
  F2: 0xffbf,
  F3: 0xffc0,
  F4: 0xffc1,
  F5: 0xffc2,
  F6: 0xffc3,
  F7: 0xffc4,
  F8: 0xffc5,
  F9: 0xffc6,
  F10: 0xffc7,
  F11: 0xffc8,
  F12: 0xffc9,
};

/** A DOM `key` → X11 keysym (undefined for dead/unknown keys). */
export function keysymFor(key: string): number | undefined {
  if (key in KEYSYMS) return KEYSYMS[key];
  const chars = [...key];
  if (chars.length !== 1) return undefined;
  const cp = chars[0].codePointAt(0)!;
  if (cp < 0x20 || (cp >= 0x7f && cp < 0xa0)) return undefined;
  if (cp <= 0xff) return cp;
  return 0x01000000 + cp;
}

export type Modifier = 'ctrl' | 'alt' | 'shift' | 'super';
export const MODIFIER_KEYSYM: Record<Modifier, number> = { ctrl: 0xffe3, alt: 0xffe9, shift: 0xffe1, super: 0xffeb };

/** A key press with held modifiers → the down/up sequence. */
export function pressCommands(keysym: number, mods: Modifier[] = []): DesktopCmd[] {
  const out: DesktopCmd[] = [];
  for (const m of mods) out.push({ op: 'key', keysym: MODIFIER_KEYSYM[m], down: true });
  out.push({ op: 'key', keysym, down: true }, { op: 'key', keysym, down: false });
  for (const m of [...mods].reverse()) out.push({ op: 'key', keysym: MODIFIER_KEYSYM[m], down: false });
  return out;
}

/**
 * A hardware keydown/keyup (desktop app) → commands. Keys map 1:1 as
 * down/up so held keys, shortcuts and modifiers just work.
 */
export function hardwareKey(e: { key: string; isComposing?: boolean }, down: boolean): DesktopCmd | undefined {
  if (e.isComposing || e.key === 'Unidentified' || e.key === 'Process' || e.key === 'Dead') return undefined;
  const keysym = keysymFor(e.key);
  return keysym === undefined ? undefined : { op: 'key', keysym, down };
}

/**
 * The soft keyboard's hidden input changed (`diffImeValue` result): erase
 * with BackSpace presses, type the rest as text (with sticky modifiers the
 * text is sent as key presses so Ctrl+C works).
 */
export function imeToCommands(bytes: string, mods: Modifier[]): DesktopCmd[] {
  const out: DesktopCmd[] = [];
  let text = '';
  const flush = () => {
    if (text) out.push({ op: 'text', text });
    text = '';
  };
  for (const ch of bytes) {
    if (ch === '\x7f') {
      flush();
      out.push(...pressCommands(KEYSYMS.Backspace, mods));
    } else if (ch === '\n') {
      flush();
      out.push(...pressCommands(KEYSYMS.Enter, mods));
    } else if (mods.length) {
      const k = keysymFor(ch);
      if (k !== undefined) out.push(...pressCommands(k, mods));
    } else {
      text += ch;
    }
  }
  flush();
  return out;
}

// ------------------------------------------------------------- transport

export type StreamMsg =
  | { kind: 'event'; event: DesktopEvent }
  | { kind: 'closed'; message?: string }
  | { kind: 'error'; message: string; code?: string; reason?: string; hint?: string };

const StreamMsgSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('event'), event: DesktopEventSchema }),
  z.object({ kind: z.literal('closed'), message: z.string().optional() }),
  z.object({ kind: z.literal('error'), message: z.string(), code: z.string().optional(), reason: z.string().optional(), hint: z.string().optional() }),
]);

export function parseStreamMsg(raw: unknown): StreamMsg | undefined {
  const parsed = StreamMsgSchema.safeParse(raw);
  return parsed.success ? (parsed.data as StreamMsg) : undefined;
}

export type OpenArgs = { viewport: DesktopViewport; quality?: DesktopQuality; viewOnly?: boolean; display?: string };

export interface ScreenTransport {
  status(hostId: string): Promise<Result<DesktopStatusData>>;
  open(hostId: string, args: OpenArgs, onMsg: (msg: StreamMsg) => void, onFrame: (bytes: Uint8Array) => void): Promise<Result<{ streamId: string }>>;
  cmd(streamId: string, cmd: DesktopCmd): Promise<void>;
  stop(streamId: string): void;
}

const OpenResultSchema = z.object({ streamId: z.string() });

export const tauriScreenTransport: ScreenTransport = {
  status: (hostId) => call('screen_status', { hostId }, DesktopStatusDataSchema),
  async open(hostId, args, onMsg, onFrame) {
    const onEvent = new Channel<unknown>();
    onEvent.onmessage = (raw) => {
      const msg = parseStreamMsg(raw);
      if (msg) onMsg(msg);
    };
    const frames = new Channel<ArrayBuffer | Uint8Array | number[]>();
    frames.onmessage = (raw) => onFrame(raw instanceof Uint8Array ? raw : new Uint8Array(raw as ArrayBuffer));
    return call('screen_open', { hostId, viewport: args.viewport, quality: args.quality, viewOnly: args.viewOnly, display: args.display, onEvent, onFrame: frames }, OpenResultSchema);
  },
  async cmd(streamId, cmd) {
    await invoke('screen_cmd', { streamId, cmd });
  },
  stop(streamId) {
    void invoke('screen_stop', { streamId }).catch(() => {});
  },
};

let override: ScreenTransport | undefined;
export function setScreenTransport(next: ScreenTransport | undefined): void {
  override = next;
}
export function transport(): ScreenTransport {
  if (override) return override;
  const injected = typeof window !== 'undefined' ? (window as { __UNIPI_SCREEN__?: ScreenTransport }).__UNIPI_SCREEN__ : undefined;
  return injected ?? tauriScreenTransport;
}

// ------------------------------------------------------------- session

export type SessionState =
  | { kind: 'connecting' }
  | { kind: 'consent' }
  | { kind: 'live'; backend: string; width: number; height: number; viewOnly: boolean; display?: string }
  | { kind: 'problem'; problem: Problem };

export type SessionCallbacks = {
  onState(state: SessionState): void;
  onConfig?(codec: string, width: number, height: number): void;
  onStats?(stats: { fps: number; kbps: number; quality: DesktopQuality; rttMs: number }): void;
  /** Decode/draw; resolve when drawn (the ack goes out then). */
  onFrame(frame: Frame): Promise<void> | void;
};

/** One live view of the PC's desktop. */
export class ScreenSession {
  private streamId: string | undefined;
  private ended = false;
  private queue: DesktopCmd[] = [];

  constructor(
    private readonly hostId: string,
    private readonly callbacks: SessionCallbacks,
    private readonly t: ScreenTransport = transport(),
  ) {}

  async open(args: OpenArgs): Promise<void> {
    this.callbacks.onState({ kind: 'connecting' });
    const result = await this.t.open(
      this.hostId,
      args,
      (msg) => this.onMsg(msg),
      (bytes) => void this.onFrameBytes(bytes),
    );
    if (this.ended) {
      if (result.ok) this.t.stop(result.value.streamId);
      return;
    }
    if (!result.ok) {
      const e = result.error;
      this.end(problemFromError({ code: e.code, message: e.message, hint: e.hint, reason: (e.detail as { reason?: string } | undefined)?.reason }));
      return;
    }
    this.streamId = result.value.streamId;
    for (const cmd of this.queue.splice(0)) void this.send(cmd);
  }

  private onMsg(msg: StreamMsg): void {
    if (this.ended) return;
    if (msg.kind === 'event') {
      const event = msg.event;
      switch (event.kind) {
        case 'consent':
          this.callbacks.onState({ kind: 'consent' });
          break;
        case 'started':
          this.callbacks.onState({ kind: 'live', backend: event.backend, width: event.width, height: event.height, viewOnly: event.view_only, display: event.display });
          break;
        case 'config':
          this.callbacks.onConfig?.(event.codec, event.width, event.height);
          break;
        case 'stats':
          this.callbacks.onStats?.({ fps: event.fps, kbps: event.kbps, quality: event.quality, rttMs: event.rtt_ms });
          break;
        case 'closed':
          this.end(problemFromClose(event.reason));
          break;
        case 'frame':
          break;
      }
    } else if (msg.kind === 'closed') {
      this.end({ kind: 'lost', message: msg.message ?? 'The PC closed the desktop stream.' });
    } else {
      this.end(problemFromError(msg));
    }
  }

  private async onFrameBytes(bytes: Uint8Array): Promise<void> {
    if (this.ended) return;
    const frame = parseFrame(bytes);
    if (!frame) return;
    try {
      await this.callbacks.onFrame(frame);
    } finally {
      void this.send({ op: 'ack', seq: frame.seq });
    }
  }

  /** Ends with a local problem (decoder failure…). */
  fail(problem: Problem): void {
    this.end(problem);
    if (this.streamId) this.t.stop(this.streamId);
  }

  private end(problem: Problem): void {
    if (this.ended) return;
    this.ended = true;
    this.callbacks.onState({ kind: 'problem', problem });
  }

  async send(cmd: DesktopCmd): Promise<void> {
    if (this.ended) return;
    if (!this.streamId) {
      if (cmd.op !== 'ack') this.queue.push(cmd);
      return;
    }
    try {
      await this.t.cmd(this.streamId, cmd);
    } catch (error) {
      const e = toAppError(error);
      this.end(problemFromError({ code: e.code, message: e.message, hint: e.hint }));
    }
  }

  stop(): void {
    this.ended = true;
    if (this.streamId) this.t.stop(this.streamId);
    this.streamId = undefined;
  }

  get isEnded(): boolean {
    return this.ended;
  }
}

// ------------------------------------------------------------- H.264 framing

/**
 * The PC sends H.264 in Annex-B (start codes, SPS/PPS in-band on key frames).
 * Chromium / Android WebView decode that directly, but WebKitGTK's WebCodecs
 * (the Linux desktop app) only takes AVC: length-prefixed NAL units plus an
 * `avcC` description from the SPS/PPS — fed Annex-B it fails every frame with
 * "Decode error". AVC works everywhere, so the viewer always converts.
 */
export function annexBNals(data: Uint8Array): Uint8Array[] {
  const out: Uint8Array[] = [];
  let start = -1;
  let i = 0;
  while (i + 2 < data.length) {
    if (data[i] === 0 && data[i + 1] === 0 && (data[i + 2] === 1 || (data[i + 2] === 0 && i + 3 < data.length && data[i + 3] === 1))) {
      const len = data[i + 2] === 1 ? 3 : 4;
      if (start >= 0) out.push(trimZeros(data.subarray(start, i)));
      i += len;
      start = i;
      continue;
    }
    i += 1;
  }
  if (start >= 0 && start < data.length) out.push(data.subarray(start));
  return out.filter((n) => n.length > 0);
}

function trimZeros(nal: Uint8Array): Uint8Array {
  let end = nal.length;
  while (end > 0 && nal[end - 1] === 0) end -= 1;
  return nal.subarray(0, end);
}

const nalType = (nal: Uint8Array) => nal[0]! & 0x1f;

/** `avcC` (AVCDecoderConfigurationRecord) from a key frame's SPS + PPS; undefined without them. */
export function avcDescription(keyFrame: Uint8Array): Uint8Array | undefined {
  const nals = annexBNals(keyFrame);
  const sps = nals.find((n) => nalType(n) === 7);
  const pps = nals.find((n) => nalType(n) === 8);
  if (!sps || !pps || sps.length < 4) return undefined;
  const out = new Uint8Array(6 + 2 + sps.length + 1 + 2 + pps.length);
  let p = 0;
  out[p++] = 1; // configurationVersion
  out[p++] = sps[1]!; // profile
  out[p++] = sps[2]!; // compatibility
  out[p++] = sps[3]!; // level
  out[p++] = 0xff; // 4-byte lengths
  out[p++] = 0xe1; // one SPS
  out[p++] = sps.length >> 8;
  out[p++] = sps.length & 0xff;
  out.set(sps, p);
  p += sps.length;
  out[p++] = 1; // one PPS
  out[p++] = pps.length >> 8;
  out[p++] = pps.length & 0xff;
  out.set(pps, p);
  return out;
}

/** Annex-B access unit → AVC (4-byte big-endian lengths), dropping SPS/PPS/AUD (they live in the description). */
export function annexBToAvc(data: Uint8Array): Uint8Array {
  const nals = annexBNals(data).filter((n) => {
    const t = nalType(n);
    return t !== 7 && t !== 8 && t !== 9;
  });
  let size = 0;
  for (const n of nals) size += 4 + n.length;
  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  let p = 0;
  for (const n of nals) {
    view.setUint32(p, n.length);
    out.set(n, p + 4);
    p += 4 + n.length;
  }
  return out;
}

/** Same SPS/PPS? (a new description means a fresh decoder configuration). */
export function sameBytes(a: Uint8Array | undefined, b: Uint8Array | undefined): boolean {
  if (!a || !b || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) if (a[i] !== b[i]) return false;
  return true;
}
