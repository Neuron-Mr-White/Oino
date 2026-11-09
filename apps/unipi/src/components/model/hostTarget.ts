/**
 * A settings target for the model dialog that needs no chat: the host's own
 * settings through `host_cmd`, with the same `_oino/*` command names a chat or
 * the New-chat draft answers. The dialog's global writes (the cycle set — pi's
 * `enabledModels` —, "Use at startup") and reads go through it from Settings.
 */
import { z } from 'zod';
import { hostCmd } from '../../settings/hostCmd';
import type { SessionHandle } from '../../lib/session/store';

type Cmd = Parameters<SessionHandle['cmd']>[0];

export function hostSettingsTarget(hostId: string): Pick<SessionHandle, 'cmd'> {
  const ok = async <T>(op: Parameters<typeof hostCmd>[0]): Promise<T> => {
    const res = await hostCmd(op, z.unknown(), hostId);
    if (!res.ok) throw res.error;
    return res.value as T;
  };
  return {
    async cmd<T = unknown>(cmd: Cmd): Promise<T> {
      const scope = cmd.scope === 'project' ? 'project' : 'global';
      if (cmd.type === '_oino/get_settings') return ok<T>({ kind: 'get_settings', ...(typeof cmd.key === 'string' ? { key: cmd.key } : {}) });
      if (cmd.type === '_oino/set_setting' && typeof cmd.key === 'string') {
        await ok({ kind: 'set_setting', key: cmd.key, value: cmd.value, scope });
        return null as T;
      }
      if (cmd.type === '_oino/get_extension_settings' && typeof cmd.namespace === 'string') {
        return ok<T>({ kind: 'get_extension_settings', namespace: cmd.namespace, layer: 'effective' });
      }
      if (cmd.type === '_oino/set_extension_setting' && typeof cmd.namespace === 'string' && typeof cmd.key === 'string') {
        await ok({ kind: 'set_extension_setting', namespace: cmd.namespace, key: cmd.key, value: cmd.value, scope });
        return null as T;
      }
      throw { code: 'E_UNSUPPORTED', message: 'Not a host setting', retryable: false };
    },
  };
}
