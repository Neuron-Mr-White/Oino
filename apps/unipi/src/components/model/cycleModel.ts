/**
 * Model cycling (P0-10, the TUI's Ctrl+P): `model.cycle` (Mod+Alt+P — Mod+P
 * is Command center · Files and Alt+P is the workflow's plan toggle) steps
 * the focused session to the next model of
 *
 * 1. the **cycle set** (pi `enabledModels`, `/scoped-models`) when one is
 *    stored — the engine's own `cycle_model` walks it (globs and `:level`
 *    suffixes included, exactly as the terminal does);
 * 2. else Fusion's curated `ranked` list (recent → leads → sidekicks) — the
 *    app steps it with `set_model` (through `fusion.set-pair` while Fusion is
 *    on) and applies the curated default effort;
 * 3. else the engine's `cycle_model` over every model (TUI parity).
 *
 * One small toast says where it landed: "Model → <name>".
 */
import type { SessionHandle } from '../../lib/session/store';
import { toasts } from '../../lib/toasts';
import { fetchFusion, fusionAvailable, setFusionActive } from '../../lib/model/fusion';
import { modelDisplayName } from '../../lib/model/displayName';
import { modelKey, parseModels, shortModelName } from '../../lib/model/picker';

/** The toast text. */
export const cycleToast = (name: string): string => `Model → ${name}`;

/** The key after `current` in `list` (wraps; the first when `current` is not in it). */
export function nextKey(list: readonly string[], current: string | undefined): string | undefined {
  if (!list.length) return undefined;
  const at = current ? list.indexOf(current) : -1;
  return list[(at + 1) % list.length];
}

const storedScope = (reply: unknown): boolean => {
  const v = (reply as { value?: unknown } | undefined)?.value;
  return Array.isArray(v) && v.some((x) => typeof x === 'string' && x.trim() !== '');
};

const busy = new Set<string>();

/** Cycles the session's model; resolves to the toast text (or `undefined` when nothing moved). */
export async function cycleSessionModel(s: SessionHandle): Promise<string | undefined> {
  if (busy.has(s.key)) return undefined;
  busy.add(s.key);
  try {
    const [scope, fusionOn] = await Promise.all([
      s.cmd<unknown>({ type: '_oino/get_settings', key: 'enabledModels' }).catch(() => undefined),
      fusionAvailable(s),
    ]);
    const fusion = !storedScope(scope) && fusionOn ? await fetchFusion(s) : undefined;
    const ranked = fusion?.curated?.ranked ?? [];
    let text: string | undefined;
    if (ranked.length) {
      const models = parseModels(await s.cmd<unknown>({ type: 'get_available_models' }).catch(() => undefined));
      const catalogue = new Map(models.map((m) => [modelKey(m), m]));
      const list = ranked.map((e) => e.key).filter((k) => catalogue.size === 0 || catalogue.has(k));
      const cur = s.state.model?.provider && s.state.model.id ? modelKey(s.state.model) : undefined;
      const key = nextKey(list, cur);
      if (!key || key === cur) return undefined;
      if (fusion?.active) await setFusionActive(s, { kind: 'single', model: key });
      else {
        const slash = key.indexOf('/');
        await s.cmd({ type: 'set_model', provider: key.slice(0, slash), modelId: key.slice(slash + 1) });
      }
      const effort = fusion?.curated?.effort[key];
      if (effort) await s.cmd({ type: 'set_thinking_level', level: effort }).catch(() => undefined);
      const m = catalogue.get(key);
      const name = (m && modelDisplayName(m)) || ranked.find((e) => e.key === key)?.name || key;
      text = cycleToast(shortModelName(name));
    } else {
      const reply = await s.cmd<{ model?: { provider?: string; id?: string; name?: string } } | null>({ type: 'cycle_model' });
      const m = reply?.model;
      if (!m) {
        toasts().push('No other model to cycle to', 'info');
        return undefined;
      }
      text = cycleToast(shortModelName(modelDisplayName(m) || m.id || ''));
    }
    toasts().push(text, 'info');
    return text;
  } catch (e) {
    toasts().push(`Could not switch model: ${String((e as { message?: unknown })?.message ?? e)}`, 'error');
    return undefined;
  } finally {
    busy.delete(s.key);
  }
}
