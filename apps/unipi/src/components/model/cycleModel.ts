/**
 * Model cycling (P0-10, F-04): `model.cycle` (Mod+Alt+P — Mod+P is Command center · Files and
 * Alt+P is the workflow's plan toggle) steps **this chat** to the next model of a short list:
 *
 * 1. the **cycle set** (`enabledModels`, `/scoped-models`) when one is stored;
 * 2. else Fusion's curated `ranked` list (recent → leads → sidekicks);
 * 3. else every usable chat model, but only when there are few of them.
 *
 * Only chat models of working providers take part (no image / speech / embedding models, no
 * dead keys), the pick never changes the PC default (E-19), and one toast says where it landed:
 * "Model → <name>".
 */
import type { SessionHandle } from '../../lib/session/store';
import { toasts } from '../../lib/toasts';
import { fetchFusion, fusionAvailable, setFusionActive } from '../../lib/model/fusion';
import { modelDisplayName } from '../../lib/model/displayName';
import { modelKey, parseModels, shortModelName } from '../../lib/model/picker';
import { cycleList } from '../../lib/model/cycle';
import { splitAvailability } from '../../lib/model/availability';
import { hostReadiness } from '../../lib/model/readiness';
import { setChatModel } from '../../lib/model/chatModel';

import { defaultSuggestedKeys, routeLabel, startupKey } from './suggested';

/** The toast text. */
export const cycleToast = (name: string): string => `Model → ${name}`;

/** The key after `current` in `list` (wraps; the first when `current` is not in it). */
export function nextKey(list: readonly string[], current: string | undefined): string | undefined {
  if (!list.length) return undefined;
  const at = current ? list.indexOf(current) : -1;
  return list[(at + 1) % list.length];
}

const storedPatterns = (reply: unknown): string[] => {
  const v = (reply as { value?: unknown } | undefined)?.value;
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && x.trim() !== '') : [];
};

const valueOf = (reply: unknown): unknown => (reply as { value?: unknown } | undefined)?.value;

/** Per-session serialized queue + optimistic cursor so rapid Ctrl+Alt+P presses step & toast every time (F-04). */
const inflight = new Map<string, Promise<unknown>>();
const optimisticModel = new Map<string, string>();

/** Cycles the session's model; resolves to the toast text (or `undefined` when nothing moved). */
export function cycleSessionModel(s: SessionHandle): Promise<string | undefined> {
  const prev = inflight.get(s.key) ?? Promise.resolve();
  const job = prev.then(() => runCycleStep(s));
  const tracked = job.finally(() => {
    if (inflight.get(s.key) === tracked) {
      inflight.delete(s.key);
      optimisticModel.delete(s.key);
    }
  });
  inflight.set(s.key, tracked);
  return job;
}

async function runCycleStep(s: SessionHandle): Promise<string | undefined> {
  try {
    const [scope, fusionOn, modelsReply, readiness, defProv, defModel] = await Promise.all([
      s.cmd<unknown>({ type: '_oino/get_settings', key: 'enabledModels' }).catch(() => undefined),
      fusionAvailable(s),
      s.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined),
      hostReadiness(s.hostId).catch(() => undefined),
      s.cmd<unknown>({ type: '_oino/get_settings', key: 'defaultProvider' }).catch(() => undefined),
      s.cmd<unknown>({ type: '_oino/get_settings', key: 'defaultModel' }).catch(() => undefined),
    ]);
    const models = parseModels(modelsReply);
    const stateCur = s.state.model?.provider && s.state.model.id ? modelKey(s.state.model) : undefined;
    const cur = optimisticModel.get(s.key) ?? stateCur;
    const usableModels = splitAvailability(models, readiness, cur).usable;
    const catalogue = new Map(usableModels.map((m) => [modelKey(m), m]));
    const fusion = fusionOn ? await fetchFusion(s).catch(() => undefined) : undefined;
    const startup = startupKey(valueOf(defProv), valueOf(defModel));
    const curatedKeys = fusion?.curated?.ranked.length
      ? fusion.curated.ranked.map((e) => e.key)
      : defaultSuggestedKeys(usableModels, cur, startup);
    const list = cycleList({
      patterns: storedPatterns(scope),
      curated: curatedKeys,
      usable: [...catalogue.keys()],
    });
    if (!list) {
      toasts().push('No usable chat models available to cycle', 'info');
      return undefined;
    }
    const key = nextKey(list, cur);
    if (!key || key === cur) {
      toasts().push('No other model to cycle to', 'info');
      return undefined;
    }
    optimisticModel.set(s.key, key);
    if (fusion?.active) await setFusionActive(s, { kind: 'single', model: key });
    else {
      const slash = key.indexOf('/');
      await setChatModel(s, key.slice(0, slash), key.slice(slash + 1));
    }
    const effort = fusion?.curated?.effort[key];
    if (effort) await s.cmd({ type: 'set_thinking_level', level: effort }).catch(() => undefined);
    const m = catalogue.get(key);
    const baseName = shortModelName((m && modelDisplayName(m)) || fusion?.curated?.ranked.find((e) => e.key === key)?.name || key);
    // D-13/F-04: if another entry in the cycle list shares the same short name, append the route so toasts stay distinct.
    const dupName = m && list.some((other) => {
      if (other === key) return false;
      const om = catalogue.get(other);
      return om && shortModelName(modelDisplayName(om)) === baseName;
    });
    const label = dupName && m ? `${baseName} (${routeLabel(m)})` : baseName;
    const text = cycleToast(label);
    toasts().push(text, 'info');
    return text;
  } catch (e) {
    optimisticModel.delete(s.key);
    toasts().push(`Could not switch model: ${String((e as { message?: unknown })?.message ?? e)}`, 'error');
    return undefined;
  }
}
