/**
 * Remote desktop browser (UNI-215, forwarding phase 3): the UI's seam onto
 * the core's `browser_status` / `browser_open` / `browser_cmd` /
 * `browser_stop` / `browser_close` (`src-tauri/src/browser.rs`) — a
 * separate Chrome on the PC (own profile), streamed as JPEG frames and driven
 * with taps, drags, keys and text over CDP.
 *
 * Pure helpers (`statusView`, `parseFrame`, `fitRect`, `toPage`,
 * `viewportFor`, `keyCommand`, `imeCommands`, `displayUrl`) carry the rules
 * so they are unit-tested; `BrowserSession` owns one live stream.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { z } from 'zod';
import {
  BrowserAgentSchema,
  BrowserProfilesDataSchema,
  BrowserEventSchema,
  BrowserOpenDataSchema,
  BrowserStatusDataSchema,
  toAppError,
  type AppError,
  type BrowserAgent,
  type BrowserCmd,
  type BrowserProfilesData,
  type BrowserEvent,
  type BrowserOpenData,
  type BrowserStatusData,
  type BrowserViewport,
} from '@unipi/protocol';
import { call, type Result } from './ipc';
import { can } from './platform';
import { diffImeValue, type ImeState } from './ime';

/** The remote browser needs the native core (iroh streams). */
export function canBrowse(): boolean {
  return can('portForward');
}

export const ENABLE_BROWSER_COMMAND = 'unipi-oino-host browser enable';
export const ENABLE_BROWSER_HEADED_COMMAND = 'unipi-oino-host browser enable --headed';
/** How to get Google Chrome onto the PC, per its OS (UNI-268: Chrome is the one we name). */
export const INSTALL_STEPS: { os: string; text: string; command?: string }[] = [
  { os: 'Linux', text: 'Download the .deb or .rpm from google.com/chrome and open it, then on Debian/Ubuntu:', command: 'sudo apt install ./google-chrome-stable_current_amd64.deb' },
  { os: 'macOS', text: 'Download it from google.com/chrome, or:', command: 'brew install --cask google-chrome' },
  { os: 'Windows', text: 'Download it from google.com/chrome, or:', command: 'winget install Google.Chrome' },
];

// ------------------------------------------------------------- status

export function browserStatus(hostId: string): Promise<Result<BrowserStatusData>> {
  return transport().status(hostId);
}
export function browserClose(hostId: string): Promise<Result<null>> {
  return transport().close(hostId);
}
/** UNI-268: show/hide the PC browser's window (hosts ≥ proto 1.13.0). */
export function browserWindow(hostId: string, show: boolean): Promise<Result<BrowserStatusData>> {
  return transport().window(hostId, show);
}

/** UNI-296: the PC's browser profiles (hosts ≥ proto 1.17.0; older ones answer E_SCHEMA/E_UNSUPPORTED). */
export function browserProfiles(hostId: string): Promise<Result<BrowserProfilesData>> {
  const t = transport();
  return t.profiles ? t.profiles(hostId) : Promise.resolve(unsupported());
}
/** UNI-296: take the PC's browser over from agents (`on`) or give it back. */
export function browserTakeover(hostId: string, on: boolean, profile?: string): Promise<Result<BrowserAgent>> {
  const t = transport();
  return t.takeover ? t.takeover(hostId, on, profile) : Promise.resolve(unsupported());
}
function unsupported<T>(): Result<T> {
  return { ok: false, error: { code: 'E_UNSUPPORTED', message: 'Not supported here', retryable: false } as AppError };
}

/** Old hosts sent `brave`, `chrome` …: what to call the browser. */
export function browserName(name: string | undefined): string {
  const n = (name ?? '').trim();
  const lower = n.toLowerCase();
  if (!n) return 'Browser';
  if (lower === 'chrome') return 'Google Chrome';
  if (lower === 'chromium') return 'Chromium';
  if (lower === 'edge') return 'Microsoft Edge';
  if (n.includes(' ') && !lower.includes('brave')) return n;
  return 'Chromium-based browser';
}

/** Can this PC show the browser window? (Older hosts don't say: assume yes, the call answers.) */
/** UNI-299: the browser has a visible window on the PC itself. Hosts that
 *  report `display` say it directly ("hidden" = only in the app, "window",
 *  "headless"); older ones only have `headless`. */
export function windowShown(data: { headless: boolean; display?: string | null }): boolean {
  if (data.display) return data.display === 'window';
  return !data.headless;
}

export function canShowWindow(status: Pick<BrowserStatusData, 'can_show_window'> | undefined): boolean {
  return status?.can_show_window !== false;
}

/** What the Browser panel shows before a session starts. */
export type StatusView =
  | { kind: 'loading' }
  | { kind: 'disabled'; error?: AppError }
  | { kind: 'not-found'; hint?: string }
  | { kind: 'error'; error: AppError }
  | { kind: 'ready'; status: BrowserStatusData };

export function statusView(result: Result<BrowserStatusData> | undefined): StatusView {
  if (!result) return { kind: 'loading' };
  if (!result.ok) {
    if (result.error.code === 'E_BROWSER_DISABLED') return { kind: 'disabled', error: result.error };
    if (result.error.code === 'E_UNSUPPORTED' || result.error.code === 'E_SCHEMA') {
      // An older unipi-host that doesn't know browser_status.
      return { kind: 'error', error: { ...result.error, message: 'This PC runs an older agent host without the remote browser.', hint: 'Update UniPi on the PC.' } };
    }
    return { kind: 'error', error: result.error };
  }
  const status = result.value;
  if (!status.enabled) return { kind: 'disabled' };
  if (!status.browser) return { kind: 'not-found', hint: status.install_hint };
  return { kind: 'ready', status };
}

/** A session-level failure → what the viewer overlay shows. */
export type SessionProblem =
  | { kind: 'disabled'; message: string }
  | { kind: 'not-found'; message: string; hint?: string }
  | { kind: 'crashed'; message: string; stderr?: string[] }
  | { kind: 'lost'; message: string }
  | { kind: 'closed'; message: string }
  /** UNI-268: the PC restarted its browser to show/hide the window — reconnect by itself. */
  | { kind: 'relaunch'; message: string };

/** The browser's last stderr lines, when an error carries them (`launch_failed`). */
function stderrOf(error: AppError): string[] | undefined {
  const stderr = (error.detail as { stderr?: unknown } | undefined)?.stderr;
  return Array.isArray(stderr) && stderr.length > 0 ? stderr.map(String) : undefined;
}

export function problemFromError(error: AppError): SessionProblem {
  const reason = (error.detail as { reason?: string } | undefined)?.reason;
  switch (error.code) {
    case 'E_BROWSER_DISABLED':
      return { kind: 'disabled', message: error.message };
    case 'E_BROWSER_UNAVAILABLE':
      if (reason === 'not_found') return { kind: 'not-found', message: error.message, hint: error.hint };
      // UNI-287: keep the browser's own words (why it died) for Details.
      return { kind: 'crashed', message: error.message, stderr: stderrOf(error) };
    default:
      return { kind: 'lost', message: error.message || 'Connection to the PC was lost.' };
  }
}

export function problemFromClose(reason: string): SessionProblem {
  if (reason === 'idle') return { kind: 'closed', message: 'The browser on the PC was closed because nobody was watching.' };
  if (reason === 'host_exit') return { kind: 'closed', message: 'The agent host stopped on the PC.' };
  if (reason === 'relaunch') return { kind: 'relaunch', message: 'Switching the window on the PC…' };
  return { kind: 'closed', message: 'The browser on the PC was closed.' };
}

// ------------------------------------------------------------- frames

/** One decoded frame from the core: 8-byte header + JPEG. */
export type Frame = { seq: number; cssWidth: number; cssHeight: number; jpeg: Uint8Array };

export function parseFrame(bytes: Uint8Array): Frame | undefined {
  if (bytes.length < 11) return undefined;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const jpeg = bytes.subarray(8);
  if (jpeg[0] !== 0xff || jpeg[1] !== 0xd8) return undefined;
  return { seq: view.getUint32(0), cssWidth: view.getUint16(4), cssHeight: view.getUint16(6), jpeg };
}

/** Builds a frame (tests, the capture mock). */
export function encodeFrame(seq: number, cssWidth: number, cssHeight: number, jpeg: Uint8Array): Uint8Array {
  const out = new Uint8Array(8 + jpeg.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, seq);
  view.setUint16(4, cssWidth);
  view.setUint16(6, cssHeight);
  out.set(jpeg, 8);
  return out;
}

export type Rect = { x: number; y: number; width: number; height: number };

/** Where a `cw×ch` page lands inside a `bw×bh` box (contain, centred). */
export function fitRect(boxWidth: number, boxHeight: number, contentWidth: number, contentHeight: number): Rect {
  if (contentWidth <= 0 || contentHeight <= 0 || boxWidth <= 0 || boxHeight <= 0) return { x: 0, y: 0, width: boxWidth, height: boxHeight };
  const scale = Math.min(boxWidth / contentWidth, boxHeight / contentHeight);
  const width = contentWidth * scale;
  const height = contentHeight * scale;
  return { x: (boxWidth - width) / 2, y: (boxHeight - height) / 2, width, height };
}

/** A point in the canvas box → page CSS px (undefined outside the page). */
export function toPage(
  px: number,
  py: number,
  box: { width: number; height: number },
  page: { width: number; height: number },
): { x: number; y: number } | undefined {
  const r = fitRect(box.width, box.height, page.width, page.height);
  const x = ((px - r.x) / r.width) * page.width;
  const y = ((py - r.y) / r.height) * page.height;
  if (x < -1 || y < -1 || x > page.width + 1 || y > page.height + 1) return undefined;
  return { x: Math.min(Math.max(x, 0), page.width), y: Math.min(Math.max(y, 0), page.height) };
}

/** Max DPR we ask the PC to render at (bandwidth). */
export const MAX_SCALE = 2;

/** The viewport for a canvas box of `w×h` CSS px. */
export function viewportFor(width: number, height: number, dpr: number, mobile: boolean): BrowserViewport {
  const clamp = (v: number) => Math.min(4096, Math.max(50, Math.round(v)));
  const scale = Math.min(MAX_SCALE, Math.max(0.5, Number.isFinite(dpr) && dpr > 0 ? dpr : 1));
  return { width: clamp(width), height: clamp(height), scale: Math.round(scale * 100) / 100, mobile };
}

export function sameViewport(a: BrowserViewport | undefined, b: BrowserViewport): boolean {
  return !!a && a.width === b.width && a.height === b.height && a.scale === b.scale && a.mobile === b.mobile;
}

// ------------------------------------------------------------- keys

/** CDP modifier bitmask from a keyboard/pointer event. */
export function modifiers(e: { altKey?: boolean; ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean }): number {
  return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
}

/** Keys we send as key events (everything else printable goes as text). */
export const SPECIAL_KEYS = new Set([
  'Enter', 'Backspace', 'Tab', 'Escape', 'Delete', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown',
  'Home', 'End', 'PageUp', 'PageDown', 'F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'F10', 'F11', 'F12',
]);

/**
 * A hardware keydown → the command to send, or undefined to let it be
 * (IME composition, bare modifiers). Printable keys without Ctrl/Alt/Meta
 * become `text` (so layouts and dead keys work); specials and shortcuts
 * become `key` presses.
 */
export function keyCommand(e: { key: string; code?: string; altKey?: boolean; ctrlKey?: boolean; metaKey?: boolean; shiftKey?: boolean; isComposing?: boolean }): BrowserCmd | undefined {
  if (e.isComposing || e.key === 'Unidentified' || e.key === 'Process' || e.key === 'Dead') return undefined;
  if (['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'AltGraph'].includes(e.key)) return undefined;
  const mods = modifiers(e);
  const shortcut = (mods & 0b0111) !== 0;
  if (SPECIAL_KEYS.has(e.key) || (shortcut && e.key.length === 1)) {
    return { op: 'key', kind: 'press', key: e.key, code: e.code || undefined, modifiers: mods };
  }
  if ([...e.key].length === 1) return { op: 'text', text: e.key };
  return undefined;
}

/**
 * The soft keyboard's hidden input changed: the commands that reproduce
 * the change on the PC (Backspaces for removed chars, then insertText).
 */
export function imeCommands(next: string, state: ImeState): { cmds: BrowserCmd[]; state: ImeState; resetValue?: string } {
  const diff = diffImeValue(next, state);
  const deletes = [...diff.bytes].filter((c) => c === '\x7f').length;
  const text = diff.bytes.replace(/\x7f/g, '');
  const cmds: BrowserCmd[] = [];
  for (let i = 0; i < deletes; i += 1) cmds.push({ op: 'key', kind: 'press', key: 'Backspace', code: 'Backspace', modifiers: 0 });
  if (text) cmds.push({ op: 'text', text: text.replace(/\n/g, '') });
  if (text.includes('\n')) cmds.push({ op: 'key', kind: 'press', key: 'Enter', code: 'Enter', modifiers: 0 });
  return { cmds, state: diff.state, resetValue: diff.resetValue };
}

/** What the URL bar shows for a page URL (no scheme for http(s)). */
export function displayUrl(url: string): string {
  if (!url || url === 'about:blank') return '';
  return url.replace(/^https?:\/\//, '').replace(/\/$/, '');
}

/** A short label for a tab chip. */
export function tabLabel(tab: { title: string; url: string }): string {
  const title = tab.title.trim();
  if (title && title !== tab.url && title !== 'about:blank') return title;
  const shown = displayUrl(tab.url);
  return shown || 'New tab';
}

// ------------------------------------------------------------- transport

export type StreamMsg =
  | { kind: 'event'; event: BrowserEvent }
  | { kind: 'closed'; message?: string }
  | { kind: 'error'; message: string; code?: string };

const StreamMsgSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('event'), event: BrowserEventSchema }),
  z.object({ kind: z.literal('closed'), message: z.string().optional() }),
  z.object({ kind: z.literal('error'), message: z.string(), code: z.string().optional() }),
]);

export function parseStreamMsg(raw: unknown): StreamMsg | undefined {
  const parsed = StreamMsgSchema.safeParse(raw);
  return parsed.success ? (parsed.data as StreamMsg) : undefined;
}

export type OpenArgs = { viewport: BrowserViewport; url?: string; target?: string; /** UNI-296: a browser profile (default "default"). */ profile?: string };

export interface BrowserTransport {
  status(hostId: string): Promise<Result<BrowserStatusData>>;
  close(hostId: string): Promise<Result<null>>;
  window(hostId: string, show: boolean): Promise<Result<BrowserStatusData>>;
  /** UNI-296 (optional: older fakes). */
  profiles?(hostId: string): Promise<Result<BrowserProfilesData>>;
  takeover?(hostId: string, on: boolean, profile?: string): Promise<Result<BrowserAgent>>;
  open(
    hostId: string,
    args: OpenArgs,
    onMsg: (msg: StreamMsg) => void,
    onFrame: (bytes: Uint8Array) => void,
  ): Promise<Result<{ streamId: string; open: BrowserOpenData }>>;
  cmd(streamId: string, cmd: BrowserCmd): Promise<void>;
  stop(streamId: string): void;
}

const OpenResultSchema = z.object({ streamId: z.string(), open: BrowserOpenDataSchema });

export const tauriBrowserTransport: BrowserTransport = {
  status: (hostId) => call('browser_status', { hostId }, BrowserStatusDataSchema),
  close: (hostId) => call('browser_close', { hostId }, z.null().catch(null)),
  window: (hostId, show) => call('browser_window', { hostId, show }, BrowserStatusDataSchema),
  profiles: (hostId) => call('browser_profiles', { hostId }, BrowserProfilesDataSchema),
  takeover: (hostId, on, profile) => call('browser_takeover', { hostId, on, profile }, BrowserAgentSchema),
  async open(hostId, args, onMsg, onFrame) {
    const onEvent = new Channel<unknown>();
    onEvent.onmessage = (raw) => {
      const msg = parseStreamMsg(raw);
      if (msg) onMsg(msg);
    };
    const frames = new Channel<ArrayBuffer | Uint8Array | number[]>();
    frames.onmessage = (raw) => onFrame(raw instanceof Uint8Array ? raw : new Uint8Array(raw as ArrayBuffer));
    return call('browser_open', { hostId, viewport: args.viewport, url: args.url, target: args.target, profile: args.profile, onEvent, onFrame: frames }, OpenResultSchema);
  },
  async cmd(streamId, cmd) {
    await invoke('browser_cmd', { streamId, cmd });
  },
  stop(streamId) {
    void invoke('browser_stop', { streamId }).catch(() => {});
  },
};

let override: BrowserTransport | undefined;
/** Tests / captures inject a fake. */
export function setBrowserTransport(next: BrowserTransport | undefined): void {
  override = next;
}
export function transport(): BrowserTransport {
  if (override) return override;
  const injected = typeof window !== 'undefined' ? (window as { __UNIPI_BROWSER__?: BrowserTransport }).__UNIPI_BROWSER__ : undefined;
  return injected ?? tauriBrowserTransport;
}

// ------------------------------------------------------------- session

export type SessionState =
  | { kind: 'starting' }
  | { kind: 'live' }
  | { kind: 'problem'; problem: SessionProblem };

export type SessionCallbacks = {
  onState(state: SessionState): void;
  onEvent(event: BrowserEvent): void;
  onFrame(frame: Frame): Promise<void> | void;
  onOpen?(open: BrowserOpenData): void;
};

/** One live view of the PC's browser. */
export class BrowserSession {
  private streamId: string | undefined;
  private ended = false;
  private queue: BrowserCmd[] = [];

  constructor(
    private readonly hostId: string,
    private readonly callbacks: SessionCallbacks,
    private readonly t: BrowserTransport = transport(),
  ) {}

  async open(args: OpenArgs): Promise<void> {
    this.callbacks.onState({ kind: 'starting' });
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
      this.end({ kind: 'problem', problem: problemFromError(result.error) });
      return;
    }
    this.streamId = result.value.streamId;
    this.callbacks.onOpen?.(result.value.open);
    this.callbacks.onState({ kind: 'live' });
    for (const cmd of this.queue.splice(0)) void this.send(cmd);
  }

  private onMsg(msg: StreamMsg): void {
    if (this.ended) return;
    if (msg.kind === 'event') {
      const event = msg.event;
      if (event.kind === 'crashed') {
        this.callbacks.onState({ kind: 'problem', problem: { kind: 'crashed', message: event.message } });
        return;
      }
      if (event.kind === 'closed') {
        this.end({ kind: 'problem', problem: problemFromClose(event.reason) });
        return;
      }
      this.callbacks.onEvent(event);
    } else if (msg.kind === 'closed') {
      this.end({ kind: 'problem', problem: { kind: 'lost', message: msg.message ?? 'The PC closed the browser view.' } });
    } else {
      const error = toAppError({ code: msg.code ?? 'E_TRANSPORT', message: msg.message, retryable: true });
      this.end({ kind: 'problem', problem: problemFromError(error) });
    }
  }

  private async onFrameBytes(bytes: Uint8Array): Promise<void> {
    if (this.ended) return;
    const frame = parseFrame(bytes);
    if (!frame) return;
    try {
      await this.callbacks.onFrame(frame);
    } finally {
      // Ack once drawn: the PC sends the next frame only then.
      void this.send({ op: 'ack', seq: frame.seq });
    }
  }

  private end(state: SessionState): void {
    if (this.ended) return;
    this.ended = true;
    this.callbacks.onState(state);
  }

  /** Sends a command (queued until the stream is open). */
  async send(cmd: BrowserCmd): Promise<void> {
    if (this.ended) return;
    if (!this.streamId) {
      if (cmd.op !== 'ack') this.queue.push(cmd);
      return;
    }
    try {
      await this.t.cmd(this.streamId, cmd);
    } catch (error) {
      this.end({ kind: 'problem', problem: problemFromError(toAppError(error)) });
    }
  }

  /** Hangs up (the PC's browser keeps running until its idle timeout). */
  stop(): void {
    this.ended = true;
    if (this.streamId) this.t.stop(this.streamId);
    this.streamId = undefined;
  }

  get isEnded(): boolean {
    return this.ended;
  }
}
