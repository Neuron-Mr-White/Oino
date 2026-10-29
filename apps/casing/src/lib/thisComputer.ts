/**
 * UNI-233: this PC's own host, managed over its local socket (desktop app):
 * who runs it (`host_info`), the paired phones (`devices_list`) and
 * unpairing one (`device_revoke`). All through the core's `local_admin`
 * (src-tauri local_host.rs), which only ever talks to THIS PC's host.
 * `unsupported` = no local socket (host not running, or older than 0.4.8).
 */
import { invoke } from '@tauri-apps/api/core';
import { createSignal } from 'solid-js';
import { DevicesListDataSchema, HostInfoDataSchema, type DeviceItem, type HostInfoData } from '@unipi/protocol';

export type ThisComputer =
  | { kind: 'loading' }
  | { kind: 'ready'; info: HostInfoData; devices: DeviceItem[] }
  | { kind: 'unsupported'; message: string }
  | { kind: 'error'; message: string };

export interface LocalAdmin {
  call(req: Record<string, unknown>): Promise<unknown>;
}
const tauriAdmin: LocalAdmin = { call: (req) => invoke('local_admin', { req }) };
let override: LocalAdmin | undefined;
export function setLocalAdmin(a: LocalAdmin | undefined): void {
  override = a;
}
const admin = () => override ?? ((window as { __UNIPI_LOCAL_ADMIN__?: LocalAdmin }).__UNIPI_LOCAL_ADMIN__ ?? tauriAdmin);

const [state, setState] = createSignal<ThisComputer>({ kind: 'loading' });
export const thisComputer = state;

function messageOf(e: unknown): { code?: string; message: string } {
  const err = e as { code?: string; message?: string };
  return { code: err?.code, message: err?.message ?? String(e) };
}

export async function refreshThisComputer(): Promise<void> {
  try {
    const info = HostInfoDataSchema.parse(await admin().call({ op: 'host_info' }));
    const devices = DevicesListDataSchema.parse(await admin().call({ op: 'devices_list' })).devices;
    setState({ kind: 'ready', info, devices: [...devices].sort((a, b) => (b.last_seen ?? b.paired_at) - (a.last_seen ?? a.paired_at)) });
  } catch (e) {
    const { code, message } = messageOf(e);
    if (code === 'E_NOT_FOUND' || code === 'E_SCHEMA' || code === 'E_UNSUPPORTED' || code === 'E_TRANSPORT') setState({ kind: 'unsupported', message });
    else setState({ kind: 'error', message });
  }
}

/** Unpair a phone; refreshes the list. Throws the host's error. */
export async function revokeDevice(id: string): Promise<void> {
  await admin().call({ op: 'device_revoke', id });
  await refreshThisComputer();
}

export const MODE_LABEL: Record<HostInfoData['mode'], string> = {
  service: 'the background service',
  app: 'this app',
  manual: 'a terminal',
};

/** "2 min ago" style, for last seen. */
export function seenText(device: DeviceItem, now = Date.now()): string {
  const at = device.last_seen ?? 0;
  if (!at) return `paired ${dayText(device.paired_at, now)}`;
  const s = Math.max(0, Math.round(now / 1000 - at));
  if (s < 90) return 'seen just now';
  if (s < 3600) return `seen ${Math.round(s / 60)} min ago`;
  if (s < 86400) return `seen ${Math.round(s / 3600)} h ago`;
  return `seen ${dayText(at, now)}`;
}

function dayText(unix: number, now: number): string {
  const days = Math.floor((now / 1000 - unix) / 86400);
  if (days <= 0) return 'today';
  if (days === 1) return 'yesterday';
  return `${days} days ago`;
}
