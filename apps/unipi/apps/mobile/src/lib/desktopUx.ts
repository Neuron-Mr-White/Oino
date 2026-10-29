/**
 * RustDesk-style desktop viewer UX (UNI-271): pure helpers for the toolbar,
 * quality presets, view styles, the phone's mouse (trackpad) mode, special
 * key chords, the zoom-aware viewport hint and remembered preferences.
 * `DesktopView.tsx` wires them to the canvas; tests cover them here.
 */
import type { DesktopCmd, DesktopQuality, DesktopStatusData } from '@unipi/protocol';
import { clampView, fitView, LONG_PRESS_MS, MODIFIER_KEYSYM, SCROLL_PX_PER_STEP, TAP_SLOP, type Size, type View } from './screen';

// ------------------------------------------------------------- quality

export type QualityOption = { value: DesktopQuality; label: string; hint: string };

/** RustDesk's three presets (hosts with `features: ['presets']`, proto 1.13). */
export const PRESET_OPTIONS: QualityOption[] = [
  { value: 'best', label: 'Best quality', hint: "The PC's full resolution · sharp text" },
  { value: 'balanced', label: 'Balanced', hint: 'Up to 2560 px · smoother' },
  { value: 'speed', label: 'Optimize reaction time', hint: 'Your screen size · lowest delay' },
];

/** Older hosts (proto ≤ 1.12) only know these. */
export const LEGACY_OPTIONS: QualityOption[] = [
  { value: 'auto', label: 'Auto', hint: 'Adapts to the connection' },
  { value: 'low', label: 'Low', hint: '≤1280 px · 15 fps' },
  { value: 'medium', label: 'Medium', hint: '≤1600 px · 24 fps' },
  { value: 'high', label: 'High', hint: '≤2560 px · 30 fps' },
];

export function hasFeature(status: Pick<DesktopStatusData, 'features'> | undefined, feature: string): boolean {
  return !!status?.features?.includes(feature);
}

export function qualityOptions(status: Pick<DesktopStatusData, 'features'> | undefined): QualityOption[] {
  return hasFeature(status, 'presets') ? PRESET_OPTIONS : LEGACY_OPTIONS;
}

/**
 * The quality to open with: the remembered one when this host knows it,
 * else "Best quality" (or the old `auto` on old hosts).
 */
export function initialQuality(status: Pick<DesktopStatusData, 'features'> | undefined, remembered?: DesktopQuality): DesktopQuality {
  const options = qualityOptions(status);
  if (remembered && options.some((o) => o.value === remembered)) return remembered;
  return options[0]!.value;
}

export function qualityLabel(q: DesktopQuality): string {
  return [...PRESET_OPTIONS, ...LEGACY_OPTIONS].find((o) => o.value === q)?.label ?? q;
}

// ------------------------------------------------------------- view style

/**
 * `adaptive` = scale to fit the window (RustDesk "Scale adaptive");
 * `original` = one PC pixel per screen pixel, pan to see the rest
 * (RustDesk "Original"); `custom` = pinch / Ctrl+wheel zoom.
 */
export type ViewStyle = 'adaptive' | 'original' | 'custom';

/** 1:1 = one PC pixel per device pixel. */
export function originalScale(dpr: number): number {
  return 1 / (Number.isFinite(dpr) && dpr > 0 ? dpr : 1);
}

/** The zoom shown in the toolbar: 100 % = 1:1 PC pixels. */
export function zoomPercent(view: View, dpr: number): number {
  return Math.round((view.scale / originalScale(dpr)) * 100);
}

/**
 * The viewport hint for the host (device px): the box, or — when zoomed in —
 * the size the PC's screen is drawn at, so the host sends enough pixels for
 * the zoom (only matters for presets capped at the viewer's size).
 */
export function viewportForView(box: Size, dpr: number, view: View | undefined, screen: Size): { width: number; height: number } {
  const d = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;
  const clamp = (v: number) => Math.min(8192, Math.max(16, Math.round(v)));
  let w = box.width * d;
  let h = box.height * d;
  if (view && screen.width > 0 && screen.height > 0) {
    w = Math.max(w, screen.width * view.scale * d);
    h = Math.max(h, screen.height * view.scale * d);
  }
  return { width: clamp(w), height: clamp(h) };
}

/**
 * Where to draw the picture on the canvas, in device pixels, snapped to
 * whole pixels so a 1:1 view is pixel-exact (fractional offsets blur text).
 */
export function drawRect(view: View, screen: Size, dpr: number): { x: number; y: number; w: number; h: number; smooth: boolean } {
  const x = Math.round(view.x * dpr);
  const y = Math.round(view.y * dpr);
  const w = Math.round(screen.width * view.scale * dpr);
  const h = Math.round(screen.height * view.scale * dpr);
  // Exactly 1:1 (or an integer upscale): nearest neighbour keeps text crisp.
  const k = view.scale * dpr;
  const integer = Math.abs(k - Math.round(k)) < 0.01 && k >= 1;
  return { x, y, w, h, smooth: !integer };
}

/** Pans so the PC's cursor (0..1) stays inside the box with a margin (CSS px). */
export function followCursor(view: View, box: Size, screen: Size, cursor: { x: number; y: number }, margin = 48): View {
  const cx = view.x + cursor.x * screen.width * view.scale;
  const cy = view.y + cursor.y * screen.height * view.scale;
  let { x, y } = view;
  const m = Math.min(margin, box.width / 4, box.height / 4);
  if (cx < m) x += m - cx;
  else if (cx > box.width - m) x -= cx - (box.width - m);
  if (cy < m) y += m - cy;
  else if (cy > box.height - m) y -= cy - (box.height - m);
  return clampView({ scale: view.scale, x, y }, box, screen);
}

/**
 * Desktop app, "Original" view of a screen bigger than the window: moving
 * the mouse near an edge pans that way (px per call, 0 when not near).
 */
export function edgePan(p: { x: number; y: number }, box: Size, edge = 36, speed = 18): { dx: number; dy: number } {
  const axis = (v: number, size: number) => (v < edge ? ((edge - v) / edge) * speed : v > size - edge ? -((v - (size - edge)) / edge) * speed : 0);
  return { dx: axis(p.x, box.width), dy: axis(p.y, box.height) };
}

export function isFit(view: View, box: Size, screen: Size): boolean {
  return Math.abs(view.scale - fitView(box, screen).scale) < 1e-6;
}

// ------------------------------------------------------------- special keys

export type SpecialKey = { id: string; label: string; keysyms: number[] };

const K = {
  ctrl: MODIFIER_KEYSYM.ctrl,
  alt: MODIFIER_KEYSYM.alt,
  shift: MODIFIER_KEYSYM.shift,
  super: MODIFIER_KEYSYM.super,
  del: 0xffff,
  esc: 0xff1b,
  tab: 0xff09,
  f4: 0xffc1,
  print: 0xff61,
  l: 0x6c,
};

/** RustDesk's "Insert Ctrl+Alt+Del" & co. */
export const SPECIAL_KEYS: SpecialKey[] = [
  { id: 'cad', label: 'Ctrl + Alt + Del', keysyms: [K.ctrl, K.alt, K.del] },
  { id: 'taskmgr', label: 'Ctrl + Shift + Esc', keysyms: [K.ctrl, K.shift, K.esc] },
  { id: 'alttab', label: 'Alt + Tab', keysyms: [K.alt, K.tab] },
  { id: 'altf4', label: 'Alt + F4', keysyms: [K.alt, K.f4] },
  { id: 'super', label: 'Super (Windows key)', keysyms: [K.super] },
  { id: 'lock', label: 'Lock screen (Super + L)', keysyms: [K.super, K.l] },
  { id: 'print', label: 'Print Screen', keysyms: [K.print] },
];

/** A chord: every key down in order, then up in reverse. */
export function chordCommands(keysyms: number[]): DesktopCmd[] {
  return [...keysyms.map((keysym) => ({ op: 'key' as const, keysym, down: true })), ...[...keysyms].reverse().map((keysym) => ({ op: 'key' as const, keysym, down: false }))];
}

/**
 * The desktop app's keyboard escape hatch: while the canvas has focus every
 * key goes to the PC, except Ctrl+Shift+F12 (releases the keyboard and
 * shows the toolbar).
 */
export function isEscapeHatch(e: { key: string; ctrlKey?: boolean; shiftKey?: boolean }): boolean {
  return e.key === 'F12' && !!e.ctrlKey && !!e.shiftKey;
}

/** Keys held on the PC (down sent, up not yet): released on blur so nothing sticks. */
export class HeldKeys {
  private held = new Set<number>();
  track(cmd: DesktopCmd): void {
    if (cmd.op !== 'key') return;
    if (cmd.down) this.held.add(cmd.keysym);
    else this.held.delete(cmd.keysym);
  }
  /** Key-ups for everything still held (and forgets them). */
  releaseAll(): DesktopCmd[] {
    const out = [...this.held].reverse().map((keysym) => ({ op: 'key' as const, keysym, down: false }));
    this.held.clear();
    return out;
  }
  get size(): number {
    return this.held.size;
  }
}

// ------------------------------------------------------------- mouse (trackpad) mode

/**
 * Phone "mouse mode" (RustDesk): the screen is a trackpad driving a cursor.
 * - one finger moves the cursor (relative, with a little acceleration)
 * - tap = left click at the cursor; tap twice = double click
 * - tap then touch again and drag = left drag (hold)
 * - long-press = right click at the cursor
 * - two-finger tap = right click; two-finger drag = scroll; pinch = zoom
 * Coordinates in: box CSS px. Cursor: 0..1 of the PC's screen.
 */
export type PadAction =
  | { kind: 'cursor'; x: number; y: number }
  | { kind: 'button'; button: 'left' | 'right' | 'middle'; down: boolean }
  | { kind: 'scroll'; dx: number; dy: number }
  | { kind: 'zoom'; factor: number; cx: number; cy: number }
  | { kind: 'pan'; dx: number; dy: number }
  | { kind: 'long-press-armed' };

type Pt = { x: number; y: number };

/** Double-tap window (ms) for "tap, then touch and drag". */
export const DOUBLE_TAP_MS = 300;

export class TrackpadMachine {
  cursor: Pt = { x: 0.5, y: 0.5 };
  private points = new Map<number, Pt>();
  private one: { id: number; start: Pt; last: Pt; time: number; moved: boolean; dragging: boolean; armDrag: boolean; longFired: boolean } | undefined;
  private lastTap: { at: Pt; time: number } | undefined;
  private two: { startDist: number; lastDist: number; lastMid: Pt; startTime: number; moved: boolean; mode?: 'scroll' | 'pinch'; travel: number } | undefined;
  private multi = false;

  /** `scale` = PC px → CSS px of the current view; `screen` = PC size. */
  constructor(private geometry: () => { scale: number; screen: Size } = () => ({ scale: 1, screen: { width: 1, height: 1 } })) {}

  setCursor(x: number, y: number): void {
    this.cursor = { x: clamp01(x), y: clamp01(y) };
  }

  down(id: number, p: Pt, time: number): PadAction[] {
    this.points.set(id, p);
    if (this.points.size === 1) {
      const armDrag = !!this.lastTap && time - this.lastTap.time < DOUBLE_TAP_MS && dist(p, this.lastTap.at) < TAP_SLOP * 4;
      this.one = { id, start: p, last: p, time, moved: false, dragging: false, armDrag, longFired: false };
      this.multi = false;
      return armDrag ? [] : [{ kind: 'long-press-armed' }];
    }
    if (this.points.size === 2) {
      const out: PadAction[] = [];
      if (this.one?.dragging) out.push({ kind: 'button', button: 'left', down: false });
      if (this.one) this.one.dragging = false;
      this.multi = true;
      const [a, b] = [...this.points.values()] as [Pt, Pt];
      const d = dist(a, b);
      this.two = { startDist: d, lastDist: d, lastMid: mid(a, b), startTime: time, moved: false, travel: 0 };
      return out;
    }
    return [];
  }

  /** The long-press timer fired: right click unless the finger moved. */
  longPress(): PadAction[] {
    const o = this.one;
    if (!o || o.moved || this.multi || o.longFired || o.armDrag) return [];
    o.longFired = true;
    return [
      { kind: 'button', button: 'right', down: true },
      { kind: 'button', button: 'right', down: false },
    ];
  }

  move(id: number, p: Pt): PadAction[] {
    if (!this.points.has(id)) return [];
    this.points.set(id, p);
    if (this.multi) {
      if (this.points.size < 2 || !this.two) return [];
      const [a, b] = [...this.points.values()] as [Pt, Pt];
      const d = dist(a, b);
      const m = mid(a, b);
      const t = this.two;
      const dm = { x: m.x - t.lastMid.x, y: m.y - t.lastMid.y };
      if (!t.mode) {
        t.travel += Math.hypot(dm.x, dm.y);
        if (Math.abs(d - t.startDist) > 24) t.mode = 'pinch';
        else if (t.travel > TAP_SLOP) t.mode = 'scroll';
      }
      const out: PadAction[] = [];
      if (t.mode === 'pinch') {
        t.moved = true;
        if (t.lastDist > 0) out.push({ kind: 'zoom', factor: d / t.lastDist, cx: m.x, cy: m.y });
        if (dm.x || dm.y) out.push({ kind: 'pan', dx: dm.x, dy: dm.y });
      } else if (t.mode === 'scroll') {
        t.moved = true;
        const dx = -dm.x / SCROLL_PX_PER_STEP;
        const dy = -dm.y / SCROLL_PX_PER_STEP;
        if (dx || dy) out.push({ kind: 'scroll', dx, dy });
      }
      t.lastDist = d;
      t.lastMid = m;
      return out;
    }
    const o = this.one;
    if (!o || o.id !== id) return [];
    const out: PadAction[] = [];
    if (!o.moved) {
      if (dist(p, o.start) < TAP_SLOP || o.longFired) return [];
      o.moved = true;
      if (o.armDrag) {
        o.dragging = true;
        out.push({ kind: 'button', button: 'left', down: true });
      }
    }
    const dx = p.x - o.last.x;
    const dy = p.y - o.last.y;
    o.last = p;
    const { scale, screen } = this.geometry();
    // Finger travel on the shown picture → PC px, faster for quick flicks.
    const speed = Math.hypot(dx, dy);
    const gain = 1.2 + Math.min(speed / 12, 1.8);
    const k = scale > 0 ? gain / scale : gain;
    this.setCursor(this.cursor.x + (dx * k) / Math.max(1, screen.width), this.cursor.y + (dy * k) / Math.max(1, screen.height));
    out.push({ kind: 'cursor', ...this.cursor });
    return out;
  }

  up(id: number, _p: Pt, time: number): PadAction[] {
    if (!this.points.has(id)) return [];
    this.points.delete(id);
    const out: PadAction[] = [];
    if (this.multi) {
      if (this.points.size === 0) {
        const t = this.two;
        if (t && !t.moved && time - t.startTime < 400) {
          out.push({ kind: 'button', button: 'right', down: true }, { kind: 'button', button: 'right', down: false });
        }
        this.reset();
        this.lastTap = undefined;
      }
      return out;
    }
    const o = this.one;
    if (o?.dragging) {
      out.push({ kind: 'button', button: 'left', down: false });
      this.lastTap = undefined;
    } else if (o && !o.moved && !o.longFired && time - o.time < 400) {
      out.push({ kind: 'button', button: 'left', down: true }, { kind: 'button', button: 'left', down: false });
      // A second quick tap is a double click, not the start of another drag.
      this.lastTap = o.armDrag ? undefined : { at: o.start, time };
    } else {
      this.lastTap = undefined;
    }
    this.reset();
    return out;
  }

  cancel(): PadAction[] {
    const out: PadAction[] = this.one?.dragging ? [{ kind: 'button', button: 'left', down: false }] : [];
    this.points.clear();
    this.reset();
    return out;
  }

  private reset() {
    this.one = undefined;
    this.two = undefined;
    this.multi = false;
  }
}

function clamp01(v: number): number {
  return Math.min(1, Math.max(0, v));
}
function dist(a: Pt, b: Pt): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}
function mid(a: Pt, b: Pt): Pt {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}

export { LONG_PRESS_MS };

// ------------------------------------------------------------- preferences

export type MouseMode = 'touch' | 'mouse';
export type DesktopPrefs = { quality?: DesktopQuality; viewStyle: 'adaptive' | 'original'; mouseMode?: MouseMode; pinned: boolean };

const PREFS_KEY = 'unipi.desktopView.v1';
const DEFAULT_PREFS: DesktopPrefs = { viewStyle: 'adaptive', pinned: false };

export function loadPrefs(storage: Pick<Storage, 'getItem'> | undefined = typeof localStorage !== 'undefined' ? localStorage : undefined): DesktopPrefs {
  try {
    const raw = storage?.getItem(PREFS_KEY);
    if (!raw) return { ...DEFAULT_PREFS };
    const v = JSON.parse(raw) as Partial<DesktopPrefs>;
    const qualities = [...PRESET_OPTIONS, ...LEGACY_OPTIONS].map((o) => o.value);
    return {
      quality: qualities.includes(v.quality as DesktopQuality) ? v.quality : undefined,
      viewStyle: v.viewStyle === 'original' ? 'original' : 'adaptive',
      mouseMode: v.mouseMode === 'touch' || v.mouseMode === 'mouse' ? v.mouseMode : undefined,
      pinned: v.pinned === true,
    };
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

export function savePrefs(prefs: DesktopPrefs, storage: Pick<Storage, 'setItem'> | undefined = typeof localStorage !== 'undefined' ? localStorage : undefined): void {
  try {
    storage?.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    /* private mode / full */
  }
}

/** Toolbar auto-hide delay when not pinned (ms). */
export const TOOLBAR_HIDE_MS = 3500;
