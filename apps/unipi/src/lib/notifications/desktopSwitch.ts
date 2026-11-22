/**
 * One desktop-notification switch (owner ruling): the notify extension's
 * "Native desktop" (`notify` › `native.enabled`) is the source of truth; the
 * app's own OS notifications (needs-you / finished, `lib/desktop`
 * `desktopNotifyOn`) follow it, so the two never disagree or double up.
 */
import { readNamespaceValues } from '../../ui/settings/data';
import { setDesktopNotifyOn } from '../desktop';
import { LOCAL_HOST } from '../host/client';

/** `native.enabled` from the notify values (nested or dotted keys); undefined when absent. Pure. */
export function nativeEnabledOf(values: unknown): boolean | undefined {
  if (!values || typeof values !== 'object') return undefined;
  const v = values as Record<string, unknown>;
  if (typeof v['native.enabled'] === 'boolean') return v['native.enabled'] as boolean;
  const native = v.native;
  if (native && typeof native === 'object' && typeof (native as Record<string, unknown>).enabled === 'boolean') {
    return (native as Record<string, boolean>).enabled;
  }
  return undefined;
}

/** Reads the PC's notify settings and makes the app's desktop notifications follow "Native desktop". */
export async function syncDesktopNotify(hostId = LOCAL_HOST): Promise<boolean | undefined> {
  const r = await readNamespaceValues('notify', 'effective', hostId).catch(() => undefined);
  const on = r?.ok ? nativeEnabledOf(r.value.values) : undefined;
  if (on !== undefined) setDesktopNotifyOn(on);
  return on;
}
