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

const busy = new Set<string>();

/** Cycles the session's model; resolves to the toast text (or `undefined` when nothing moved). */
export async function cycleSessionModel(s: SessionHandle): Promise<string | undefined> {
  if (busy.has(s.key)) return undefined;
  busy.add(s.key);
  try {
    const [scope, fusionOn, modelsReply, readiness] = await Promise.all([
      s.cmd<unknown>({ type: '_oino/get_settings', key: 'enabledModels' }).catch(() => undefined),
      fusionAvailable(s),
      s.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined),
      hostReadiness(s.hostId).catch(() => undefined),
    ]);
    const models = parseModels(modelsReply);
    const cur = s.state.model?.provider && s.state.model.id ? modelKey(s.state.model) : undefined;
    const usableModels = splitAvailability(models, readiness, cur).usable;
    const catalogue = new Map(usableModels.map((m) => [modelKey(m), m]));
    const fusion = fusionOn ? await fetchFusion(s).catch(() => undefined) : undefined;
    const list = cycleList({
      patterns: storedPatterns(scope),
      curated: (fusion?.curated?.ranked ?? []).map((e) => e.key),
      usable: [...catalogue.keys()],
    });
    if (!list) {
      toasts().push('Choose the models to cycle in the model picker (Cycle set)', 'info');
      return undefined;
    }
    const key = nextKey(list, cur);
    if (!key || key === cur) {
      toasts().push('No other model to cycle to', 'info');
      return undefined;
    }
    if (fusion?.active) await setFusionActive(s, { kind: 'single', model: key });
    else {
      const slash = key.indexOf('/');
      await setChatModel(s, key.slice(0, slash), key.slice(slash + 1));
    }
    const effort = fusion?.curated?.effort[key];
    if (effort) await s.cmd({ type: 'set_thinking_level', level: effort }).catch(() => undefined);
    const m = catalogue.get(key);
    const name = (m && modelDisplayName(m)) || fusion?.curated?.ranked.find((e) => e.key === key)?.name || key;
    const text = cycleToast(shortModelName(name));
    toasts().push(text, 'info');
    return text;
  } catch (e) {
    toasts().push(`Could not switch model: ${String((e as { message?: unknown })?.message ?? e)}`, 'error');
    return undefined;
  } finally {
    busy.delete(s.key);
  }
}
