/**
 * Typed wrappers around the M1 host/pairing/session Tauri commands
 * (DESIGN.md §3.2, §3.3, §5.3). Every call goes through `call()`
 * (lib/ipc.ts), so every result is zod-parsed before the UI sees it.
 */
import { z } from 'zod';
import { call, type Result } from './ipc';
import type { AppError } from '@unipi/protocol';
import {
  HostInfoSchema,
  HostStatusSchema,
  MuxPreviewResultSchema,
  MuxTreeResultSchema,
  SessionsResultSchema,
  type HostInfo,
  type HostStatus,
  type MuxPreviewResult,
  type MuxTreeResult,
  type SessionsResult,
} from './schemas';
import type { Relay } from '../settings/schema';
import type { MuxKind } from '@unipi/protocol';

const ListHostsSchema = z.array(HostInfoSchema);

/** Pairs with a host from a scanned/pasted `unipi://pair` link. */
export function pairHost(args: { link: string; device_name: string; relay?: Relay }): Promise<Result<HostInfo, AppError>> {
  return call('pair_host', args, HostInfoSchema);
}

/** Lists every paired host. */
export function listHosts(): Promise<Result<HostInfo[], AppError>> {
  return call('list_hosts', {}, ListHostsSchema);
}

/** Forgets a paired host by id. */
export function forgetHost(id: string): Promise<Result<unknown, AppError>> {
  return call('forget_host', { id }, z.unknown());
}

/** Lists herdr/tmux sessions on a paired host. */
export function muxTree(args: { host_id: string; relay?: Relay }): Promise<Result<MuxTreeResult, AppError>> {
  return call('mux_tree', args, MuxTreeResultSchema);
}
export function muxPreview(args: {
  host_id: string;
  relay?: Relay;
  target: import('@unipi/protocol').AttachTarget;
  lines: number;
}): Promise<Result<MuxPreviewResult, AppError>> {
  return call('mux_preview', args, MuxPreviewResultSchema);
}
export function listSessions(args: { host_id: string; mux: MuxKind; relay?: Relay }): Promise<Result<SessionsResult, AppError>> {
  return call('list_sessions', args, SessionsResultSchema);
}

/** Checks reachability/round-trip time to a paired host. */
export function hostStatus(host_id: string): Promise<Result<HostStatus, AppError>> {
  return call('host_status', { host_id }, HostStatusSchema);
}
