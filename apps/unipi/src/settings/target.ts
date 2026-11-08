/**
 * Which PC the host-scoped Settings sections talk to (phone spec §3.9, PH-04).
 *
 * The desktop app manages its own agent host: everything stays on `LOCAL_HOST`
 * (unchanged). A phone (or a browser) has no local host — `local` resolves to
 * nothing and every section said "Host not found" — so there the sections follow
 * the globally selected PC (`hosts.selectedId()`), falling back to the most
 * recently paired one.
 */
import { createEffect, createSignal } from 'solid-js';
import { LOCAL_HOST } from '../lib/host/client';
import { resolveHostId } from '../lib/host/localId';
import type { HostsApi } from '../hosts/context';

const [target, setTarget] = createSignal<string>(LOCAL_HOST);

/** The host id Settings' host commands default to. */
export function settingsHostId(): string {
  return target();
}

/** True when Settings manages a paired PC rather than this computer. */
export function settingsTargetsRemote(): boolean {
  return target() !== LOCAL_HOST;
}

/** Pure rule (tested): this computer when there is one, else the selected PC. */
export function resolveSettingsHost(hosts: ReadonlyArray<{ id: string; transport?: string }>, selected: string | undefined): string {
  if (hosts.length === 0 || hosts.some((h) => h.transport === 'local')) return LOCAL_HOST;
  return (selected && hosts.some((h) => h.id === selected) ? selected : hosts[0]?.id) ?? LOCAL_HOST;
}

/** Keeps the target in step with the hosts store (mounted once, app-wide). */
export function syncSettingsHost(hosts: Pick<HostsApi, 'hosts' | 'selectedId'>): void {
  createEffect(() => setTarget(resolveSettingsHost(hosts.hosts(), hosts.selectedId())));
}

/** Tests only. */
export function setSettingsHostForTest(id: string): void {
  setTarget(id);
}

/**
 * [sidebar-ui] A project the Settings forms are scoped to (the sidebar's "Project settings"): the
 * extension forms open on the project layer and read/write `<cwd>/.oino/settings.json`.
 * Cleared by Settings' "Back to global" or by opening Settings the normal way.
 */
const [projectScope, setProjectScope] = createSignal<{ hostId: string; cwd: string } | undefined>();
export { projectScope, setProjectScope };

/** The project cwd extension-settings calls address (`undefined` = the host's primary cwd). */
export function settingsProjectCwd(hostId = target()): string | undefined {
  const scope = projectScope();
  // The sidebar passes This computer's real id, Settings says `local`: compare resolved ids.
  return scope && resolveHostId(scope.hostId) === resolveHostId(hostId) ? scope.cwd : undefined;
}
