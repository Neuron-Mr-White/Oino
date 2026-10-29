/**
 * Sharing with phones (UNI-248): one click instead of a command.
 *
 * - **This computer** (the desktop app, viewing the PC it runs on): switches
 *   for web-app forwarding, the remote browser, desktop streaming and push.
 *   Writes go through the PC's own `unipi-host` CLI (`sharing_change` runs a
 *   whitelisted `unipi-host <feature> enable|disable …`), the same code path
 *   as typing the command; `unipi-host serve` re-reads config.json on every
 *   request, so nothing restarts. "This computer" = the host id of a paired
 *   PC equals the PUBLIC endpoint id in the local `<host dir>/endpoint.json`
 *   (`local_host`; never the secret key).
 * - **Any other PC** (phones, or the desktop app viewing another PC): "Ask
 *   the PC to turn this on" → `feature_request` (proto 1.11). The PC's owner
 *   approves or denies it (desktop app dialog, ntfy, `unipi-host requests`);
 *   nothing is switched on by itself. The asking screen polls
 *   `feature_requests` while the request is pending.
 *
 * Every call goes through `call()` (lib/ipc), so tests and the capture
 * mocks answer them like any other command.
 */
import { createRoot, createSignal } from 'solid-js';
import { z } from 'zod';
import {
  FeatureLeaveDataSchema,
  FeatureRequestItemSchema,
  FeatureRequestsDataSchema,
  type AppError,
  type FeatureKind,
  type FeatureLeaveData,
  type FeatureLeaveScope,
  type FeatureRequestItem,
  type FeatureScope,
} from '@unipi/protocol';
import { call, type Result } from './ipc';
import { can } from './platform';

export type { FeatureKind, FeatureLeaveData, FeatureLeaveScope, FeatureRequestItem, FeatureScope };

export const LocalHostSchema = z.object({
  hostDir: z.string(),
  hostId: z.string().optional(),
  bin: z.string().optional(),
});
export type LocalHost = z.infer<typeof LocalHostSchema>;

const RequestViewSchema = FeatureRequestItemSchema.extend({ device_id: z.string() });
export type RequestView = z.infer<typeof RequestViewSchema>;

export const SharingStatusSchema = z.object({
  version: z.string(),
  host_dir: z.string(),
  host_id: z.string().optional(),
  forward: z.object({ enabled: z.boolean(), any: z.boolean(), ports: z.array(z.number().int()).default([]), devices: z.array(z.string()).default([]) }),
  browser: z.object({
    enabled: z.boolean(),
    headless: z.boolean(),
    idle_minutes: z.number().int(),
    path: z.string().optional(),
    found: z.object({ name: z.string(), path: z.string(), version: z.string().optional() }).optional(),
    devices: z.array(z.string()).default([]),
  }),
  desktop: z.object({
    enabled: z.boolean(),
    view_only: z.boolean(),
    devices: z.array(z.string()).default([]),
    display: z.string(),
    reason: z.string().optional(),
  }),
  push: z.object({ enabled: z.boolean(), devices: z.array(z.string()).default([]) }),
  /** UNI-317/UNI-318: hosts before this milestone don't send it. */
  cron: z.object({ enabled: z.boolean(), devices: z.array(z.string()).default([]) }).optional(),
  devices: z.array(z.object({ id: z.string(), name: z.string() })).default([]),
  requests: z.array(RequestViewSchema).default([]),
  linger: z.boolean().optional(),
  /** UNI-263: self-update (hosts before 0.4.4 don't send it). */
  update: z
    .object({
      mode: z.enum(['prompt', 'auto']).catch('prompt'),
      latest: z.string().optional(),
      available: z.boolean(),
      checked_at: z.number().optional(),
      error: z.string().optional(),
    })
    .optional(),
});
export type SharingStatus = z.infer<typeof SharingStatusSchema>;

/** One change the desktop app may make (mirrors src-tauri `SharingChange`). */
export type SharingChange =
  | { kind: 'forward_enable'; any: boolean; ports?: number[] }
  | { kind: 'browser_enable'; headed: boolean; path?: string }
  | { kind: 'desktop_enable'; view_only: boolean }
  | { kind: 'push_enable' }
  | { kind: 'cron_enable' }
  | { kind: 'disable'; feature: FeatureKind }
  | { kind: 'approve'; id: string; for?: ApproveFor }
  | { kind: 'deny'; id: string }
  | { kind: 'linger_enable' }
  | { kind: 'host_update' }
  | { kind: 'host_update_mode'; auto: boolean };

export type ApproveFor = '15m' | '1h' | '8h' | '1d';

export const FEATURE_LABEL: Record<FeatureKind, string> = {
  forward: 'Web apps',
  browser: 'Browser',
  desktop: 'Desktop',
  push: 'Files to phones',
  cron: 'Scheduled runs',
};

/** What a feature does, one line (Settings rows, the owner's dialog). */
export const FEATURE_HELP: Record<FeatureKind, string> = {
  forward: "Open web apps running on this PC's loopback (dev servers) on your phone",
  browser: 'A separate browser on this PC that phones can drive — never your own browser or its logins',
  desktop: "See (and control) this PC's screen from a phone",
  push: 'Let `unipi-host push <file>` send files to your phones',
  cron: "See and manage this PC's scheduled, unattended pi runs",
};

/** The command a PC owner can still type (shown under "Or on the PC"). */
export const ENABLE_COMMANDS: Record<FeatureKind, string> = {
  forward: 'unipi-host forward enable --any',
  browser: 'unipi-host browser enable',
  desktop: 'unipi-host desktop enable',
  push: 'unipi-host push enable',
  cron: 'unipi-host cron share',
};

export const INSTALL_COMMAND = 'curl -fsSL https://unipi.nrn.one/install.sh | sh';

// ------------------------------------------------------------- transport

export interface SharingTransport {
  localHost(): Promise<Result<LocalHost>>;
  status(): Promise<Result<SharingStatus>>;
  change(change: SharingChange): Promise<Result<SharingStatus>>;
  ask(hostId: string, feature: FeatureKind, scope?: FeatureScope, note?: string): Promise<Result<FeatureRequestItem>>;
  requests(hostId: string): Promise<Result<FeatureRequestItem[]>>;
  /** UNI-253: turn a shared feature off / disconnect. Never enables anything. */
  leave(hostId: string, feature: FeatureKind, scope: FeatureLeaveScope): Promise<Result<FeatureLeaveData>>;
}

const ipcTransport: SharingTransport = {
  localHost: () => call('local_host', undefined, LocalHostSchema),
  status: () => call('sharing_status', undefined, SharingStatusSchema),
  change: (change) => call('sharing_change', { change }, SharingStatusSchema),
  ask: (hostId, feature, scope, note) => call('feature_request', { hostId, feature, scope, note }, FeatureRequestItemSchema),
  requests: async (hostId) => {
    const r = await call('feature_requests', { hostId }, FeatureRequestsDataSchema);
    return r.ok ? { ok: true, value: r.value?.requests ?? [] } : r;
  },
  // `feature_leave` travels the pairing (proto 1.15); the host answers with what
  // is left of the feature afterwards.
  leave: (hostId, feature, scope) => call('feature_leave', { hostId, feature, scope }, FeatureLeaveDataSchema),
};

let override: SharingTransport | undefined;
export function setSharingTransport(t: SharingTransport | undefined): void {
  override = t;
  resetSharingStore();
}
export function sharingTransport(): SharingTransport {
  return override ?? ipcTransport;
}

// ------------------------------------------------------------- store

function makeStore() {
  return createRoot(() => {
    const [local, setLocal] = createSignal<LocalHost | undefined>();
    const [status, setStatus] = createSignal<Result<SharingStatus> | undefined>();
    return { local, setLocal, status, setStatus };
  });
}
let store = makeStore();
let localLoad: Promise<LocalHost | undefined> | undefined;

export function resetSharingStore(): void {
  store = makeStore();
  localLoad = undefined;
}

/** The local host (desktop app only), loaded once. */
export function loadLocalHost(): Promise<LocalHost | undefined> {
  if (!can('localSharing')) return Promise.resolve(undefined);
  localLoad ??= sharingTransport()
    .localHost()
    .then((r) => {
      const value = r.ok ? r.value : undefined;
      store.setLocal(value);
      return value;
    });
  return localLoad;
}

export const localHost = () => store.local();

/** Pure: is the paired PC `hostId` the one this app runs on? */
export function sameHost(hostId: string | undefined, local: LocalHost | undefined): boolean {
  return !!hostId && !!local?.hostId && hostId.toLowerCase() === local.hostId.toLowerCase();
}

/** Reactive: the viewed PC is this computer (desktop app only). */
export function isThisComputer(hostId: string | undefined): boolean {
  if (!can('localSharing')) return false;
  void loadLocalHost();
  return sameHost(hostId, store.local());
}

export const sharingStatus = () => store.status();

export async function refreshSharing(): Promise<Result<SharingStatus>> {
  const r = await sharingTransport().status();
  store.setStatus(r);
  return r;
}

export async function changeSharing(change: SharingChange): Promise<Result<SharingStatus>> {
  const r = await sharingTransport().change(change);
  if (r.ok) store.setStatus(r);
  return r;
}

/**
 * UNI-253: turn a shared feature off, or disconnect this phone from it.
 * Refreshes the status afterwards so the rows stop offering it.
 */
export async function leaveFeature(hostId: string, feature: FeatureKind, scope: FeatureLeaveScope): Promise<Result<FeatureLeaveData>> {
  const r = await sharingTransport().leave(hostId, feature, scope);
  if (r.ok) await refreshSharing();
  return r;
}

// ------------------------------------------------------------- pure helpers

/** Is a feature on (for anyone) in a status? */
export function featureOn(s: SharingStatus, feature: FeatureKind): boolean {
  switch (feature) {
    case 'forward':
      return s.forward.enabled && (s.forward.any || s.forward.ports.length > 0);
    case 'browser':
      return s.browser.enabled;
    case 'desktop':
      return s.desktop.enabled;
    case 'push':
      return s.push.enabled;
    case 'cron':
      return s.cron?.enabled ?? false;
  }
}

/** "Only Pixel" / "All paired devices" for a feature's device list. */
export function devicesLine(s: SharingStatus, devices: string[]): string {
  if (devices.length === 0) return 'All paired devices';
  const names = devices.map((d) => {
    const known = s.devices.find((p) => p.id === d || (d.length >= 8 && p.id.startsWith(d)))?.name;
    if (known) return known;
    // A raw endpoint id of a device that isn't paired any more: keep it short.
    return /^[0-9a-f]{16,}$/i.test(d) ? `${d.slice(0, 8)}… (not paired)` : d;
  });
  return `Only ${names.join(', ')}`;
}

/** Parse "5173, 8080" into ports; `undefined` + message when bad. */
export function parsePorts(text: string): { ok: true; ports: number[] } | { ok: false; message: string } {
  const parts = text.split(/[\s,]+/).filter(Boolean);
  if (parts.length === 0) return { ok: false, message: 'Type at least one port, e.g. 5173' };
  const ports: number[] = [];
  for (const part of parts) {
    if (!/^\d+$/.test(part)) return { ok: false, message: `"${part}" isn't a port number` };
    const n = Number(part);
    if (n < 1 || n > 65535) return { ok: false, message: `${n} isn't a port (1–65535)` };
    if (!ports.includes(n)) ports.push(n);
  }
  if (ports.length > 16) return { ok: false, message: 'At most 16 ports' };
  return { ok: true, ports };
}

/** Pending requests, newest first. */
export function pendingRequests(s: SharingStatus | undefined): RequestView[] {
  return (s?.requests ?? []).filter((r) => r.status === 'pending').sort((a, b) => b.created_at - a.created_at);
}

/** Plain words: "watch only", "ports 5173, 8080", "window shown on the PC". */
export function scopeText(feature: FeatureKind, scope: FeatureScope | undefined): string | undefined {
  if (!scope) return undefined;
  if (feature === 'forward') {
    if (scope.any_port) return 'any port';
    if (scope.ports?.length) return `port${scope.ports.length === 1 ? '' : 's'} ${scope.ports.join(', ')}`;
  }
  if (feature === 'desktop') return scope.view_only ? 'watch only' : 'mouse and keyboard';
  if (feature === 'browser' && scope.headed) return 'window shown on the PC';
  return undefined;
}

export function requestTitle(r: Pick<FeatureRequestItem, 'device_name' | 'feature' | 'scope'>): string {
  const scope = scopeText(r.feature, r.scope);
  return `${r.device_name} asks for ${FEATURE_LABEL[r.feature].toLowerCase()}${scope ? ` (${scope})` : ''}`;
}

/** "3 min ago" / "just now". */
export function ago(ms: number, now = Date.now()): string {
  const min = Math.floor(Math.max(0, now - ms) / 60_000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min} min ago`;
  const h = Math.floor(min / 60);
  return h < 24 ? `${h} h ago` : `${Math.floor(h / 24)} d ago`;
}

/** "for 52 more min" / "until turned off". */
export function untilText(until: number | undefined, now = Date.now()): string {
  if (!until) return 'until it is turned off';
  const min = Math.max(1, Math.round((until - now) / 60_000));
  return min < 90 ? `for ${min} more min` : `for ${Math.round(min / 60)} more h`;
}

/** The asking screen's state for one feature. */
export type AskState =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | { kind: 'pending'; request: FeatureRequestItem }
  | { kind: 'approved'; request: FeatureRequestItem }
  | { kind: 'denied'; request: FeatureRequestItem }
  | { kind: 'expired'; request: FeatureRequestItem }
  | { kind: 'old-host' }
  | { kind: 'error'; error: AppError };

/** What the latest request means for the ask card. A denied/expired answer is
 * shown once; asking again starts over. */
export function askStateFrom(request: FeatureRequestItem | undefined): AskState {
  if (!request) return { kind: 'idle' };
  return { kind: request.status, request } as AskState;
}

/** A host older than proto 1.11 doesn't know the op. */
export function isOldHost(error: AppError): boolean {
  return error.code === 'E_SCHEMA' && /feature_request|unknown variant|op/i.test(error.message);
}

/** A host older than proto 1.15 doesn't know `feature_leave`. */
export function isLeaveUnsupported(error: AppError): boolean {
  return error.code === 'E_SCHEMA' && /feature_leave|unknown variant/i.test(error.message);
}

/**
 * The two confirm rows UNI-253 offers for a feature. A phone may always remove
 * *itself*; turning the feature off for everyone is only offered when this
 * device actually has it (the host refuses otherwise).
 */
export function leaveScopes(feature: FeatureKind, allowed: boolean): { scope: FeatureLeaveScope; label: string; detail: string }[] {
  const name = FEATURE_LABEL[feature].toLowerCase();
  const rows: { scope: FeatureLeaveScope; label: string; detail: string }[] = [
    { scope: 'self', label: `Turn off ${name} for this phone`, detail: 'This phone stops using it. Other phones keep what they have, and you can ask again later.' },
  ];
  if (allowed) {
    rows.push({ scope: 'all', label: `Turn off ${name} on the PC`, detail: 'It goes off for every phone. You turn it back on from the PC.' });
  }
  return rows;
}

/** Poll interval while a request is pending (ms). */
export const ASK_POLL_MS = 4000;
/** The desktop app's owner-side poll (ms). */
export const OWNER_POLL_MS = 6000;
