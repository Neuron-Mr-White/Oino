/**
 * Port forwarding (UNI-213): the UI's seam onto the Tauri core's
 * `ports_list` / `forward_start` / `forward_stop` / `forward_list`
 * (`src-tauri/src/forward.rs`). A forward is a loopback listener on this
 * device; each connection rides its own iroh stream to the PC's
 * `127.0.0.1:<port>` (`unipi-host` `tcp_forward`), so the Preview page can
 * load `http://127.0.0.1:<localPort>/` in a frame — HTTP, WebSockets and
 * Vite HMR included.
 *
 * Pure helpers (`pickerState`, `viewerBanner`, `parsePortInput`,
 * `previewUrl`) carry the UX rules so they are unit-tested.
 */
import { z } from 'zod';
import { AppErrorSchema, PortsListDataSchema, type AppError, type ListeningPort, type PortsListData } from '@unipi/protocol';
import type { Result } from './ipc';
import { appCall, hostCall } from './host/factory';
import { can } from './platform';

/** Forwarding needs the native core (a loopback listener): lib/platform.ts `portForward`. */
export function canForward(): boolean {
  return can('portForward');
}

/** The exact command the PC owner runs to switch forwarding on. */
export const ENABLE_COMMAND = 'unipi-host forward enable --any';
export const ENABLE_PORT_COMMAND = (port: number) => `unipi-host forward enable --port ${port}`;

export const ForwardInfoSchema = z.object({
  id: z.string(),
  hostId: z.string(),
  remotePort: z.number().int(),
  localPort: z.number().int(),
  url: z.string(),
  startedAt: z.number(),
  active: z.number().int().default(0),
  total: z.number().int().default(0),
  bytesUp: z.number().default(0),
  bytesDown: z.number().default(0),
  lastOk: z.number().nullish(),
  lastError: z.object({ error: AppErrorSchema, at: z.number() }).nullish(),
});
export type ForwardInfo = z.infer<typeof ForwardInfoSchema>;

export function portsList(hostId: string): Promise<Result<PortsListData>> {
  return hostCall(hostId, 'ports_list', {}, PortsListDataSchema);
}
export function forwardStart(hostId: string, port: number, localPort?: number): Promise<Result<ForwardInfo>> {
  return hostCall(hostId, 'forward_start', { port, localPort }, ForwardInfoSchema);
}
export function forwardStop(id: string): Promise<Result<boolean>> {
  return appCall('forward_stop', { id }, z.boolean());
}
export function forwardList(): Promise<Result<ForwardInfo[]>> {
  return appCall('forward_list', undefined, z.array(ForwardInfoSchema));
}

/** What the port picker shows. */
export type PickerState =
  | { kind: 'loading' }
  | { kind: 'disabled'; error: AppError }
  | { kind: 'error'; error: AppError }
  | { kind: 'empty'; any: boolean; allowed: number[] }
  | { kind: 'ports'; web: ListeningPort[]; other: ListeningPort[]; any: boolean; allowed: number[] };

export function pickerState(result: Result<PortsListData> | undefined): PickerState {
  if (!result) return { kind: 'loading' };
  if (!result.ok) {
    if (result.error.code === 'E_FORWARD_DISABLED') return { kind: 'disabled', error: result.error };
    if (result.error.code === 'E_UNSUPPORTED' || result.error.code === 'E_SCHEMA') {
      // An older unipi-host that doesn't know ports_list (seen on the Redmi: the raw serde
      // "unknown variant `ports_list`, expected one of …" filled the page).
      return { kind: 'error', error: { ...result.error, message: 'This PC runs an older unipi-host without port forwarding.', hint: 'Update unipi-host on the PC.' } };
    }
    return { kind: 'error', error: result.error };
  }
  const { ports, any, allowed } = result.value;
  if (ports.length === 0) return { kind: 'empty', any, allowed };
  // Forwardable web servers first, then everything else.
  const rank = (p: ListeningPort) => (p.allowed ? 0 : 2) + (p.http ? 0 : 1);
  const sorted = [...ports].sort((a, b) => rank(a) - rank(b) || a.port - b.port);
  return {
    kind: 'ports',
    web: sorted.filter((p) => p.http),
    other: sorted.filter((p) => !p.http),
    any,
    allowed,
  };
}

/** Parses the manual port box: an integer 1..65535 (or an error to show). */
export function parsePortInput(raw: string): { ok: true; port: number } | { ok: false; message: string } {
  const text = raw.trim().replace(/^:/, '');
  if (!text) return { ok: false, message: 'Enter a port number' };
  if (!/^\d+$/.test(text)) return { ok: false, message: 'Ports are numbers, like 5173' };
  const port = Number(text);
  if (port < 1 || port > 65535) return { ok: false, message: 'Ports go from 1 to 65535' };
  return { ok: true, port };
}

/** Builds the frame URL for a path typed in the URL bar. */
export function previewUrl(localPort: number, path: string): string {
  let p = path.trim();
  // Accept a pasted full localhost URL: keep only its path.
  const full = p.match(/^https?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])(?::\d+)?(\/.*)?$/i);
  if (full) p = full[1] ?? '/';
  if (!p.startsWith('/')) p = `/${p}`;
  return `http://127.0.0.1:${localPort}${p}`;
}

/** The path shown in the URL bar for a frame URL. */
export function urlPath(url: string): string {
  const m = url.match(/^https?:\/\/[^/]+(\/.*)?$/);
  return m ? m[1] ?? '/' : url;
}

/** Banner over the viewer when the latest connection failed. */
export type ViewerBanner =
  | { kind: 'closed'; message: string }
  | { kind: 'lost'; message: string }
  | { kind: 'disabled'; message: string; hint?: string }
  | { kind: 'error'; message: string };

export function viewerBanner(info: ForwardInfo | undefined, stopped: boolean): ViewerBanner | undefined {
  if (stopped || !info) return { kind: 'lost', message: 'This preview was stopped.' };
  const last = info.lastError;
  if (!last) return undefined;
  // A later successful connection clears an old failure.
  if (info.lastOk && info.lastOk > last.at) return undefined;
  switch (last.error.code) {
    case 'E_NOT_FOUND':
      return { kind: 'closed', message: `Nothing is listening on port ${info.remotePort} on the PC any more.` };
    case 'E_TRANSPORT':
    case 'E_TIMEOUT':
      return { kind: 'lost', message: 'Connection to the PC was lost.' };
    case 'E_FORWARD_DISABLED':
    case 'E_UNAUTHORIZED':
      return { kind: 'disabled', message: last.error.message, hint: last.error.hint };
    default:
      return { kind: 'error', message: last.error.message };
  }
}

/** Short label for a port row: process name or a fallback. */
export function portLabel(p: ListeningPort): string {
  return p.process ?? (p.http ? 'Web server' : 'Unknown process');
}
