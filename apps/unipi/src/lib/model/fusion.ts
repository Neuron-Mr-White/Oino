/**
 * Fusion pairing data (WP-A14, Z-76; extension `unipi.fusion`): the one
 * surface the app reads the lead/sidekick pair from and sets it through —
 * services `fusion.status` and `fusion.set-pair` (`_oino/service_call`).
 * The app holds no Fusion state of its own: the extension's `Active` record
 * is the truth, the picker only shows it and asks for a change.
 */
import type { SessionHandle } from '../session/store';

/** Extension id gating the Fusion tab. */
export const FUSION_EXTENSION_ID = 'unipi.fusion';
/** Point services of `unipi.fusion` (unipi-kit `services::point`). */
export const FUSION_STATUS = 'fusion.status';
export const FUSION_SET_PAIR = 'fusion.set-pair';

/** `preset::Active` — one model, or lead + sidekick. */
export type FusionActive =
  | { kind: 'single'; model: string }
  | { kind: 'fusion'; lead: string; sidekick: string; leadEffort?: string; sidekickEffort?: string };

/** `topics::FusionStatus` (camelCase): the pair's names and effort. */
export type FusionStatus = {
  leadName: string;
  leadKey?: string;
  sidekickName: string;
  sidekickKey?: string;
  leadEffort: string;
  sidekickEffort: string;
  savedUsd?: number;
  busy?: boolean;
};

/** `{active, status}` from `fusion.status`. */
export type FusionState = { active?: FusionActive; status?: FusionStatus };

const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);
const obj = (v: unknown): Record<string, unknown> | undefined =>
  v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined;

/** The extension's active selection (`{kind:'single'|'fusion', …}`), leniently. */
export function parseFusionActive(raw: unknown): FusionActive | undefined {
  const o = obj(raw) ?? obj(obj(raw)?.active);
  if (!o) return undefined;
  if (o.kind === 'single') {
    const model = str(o.model);
    return model ? { kind: 'single', model } : undefined;
  }
  if (o.kind === 'fusion') {
    const lead = str(o.lead);
    const sidekick = str(o.sidekick);
    if (!lead || !sidekick) return undefined;
    return {
      kind: 'fusion',
      lead,
      sidekick,
      ...(str(o.leadEffort) ? { leadEffort: str(o.leadEffort) } : {}),
      ...(str(o.sidekickEffort) ? { sidekickEffort: str(o.sidekickEffort) } : {}),
    };
  }
  return undefined;
}

/** The Fusion status topic/service payload. */
export function parseFusionStatus(raw: unknown): FusionStatus | undefined {
  const o = obj(raw) ?? obj(obj(raw)?.status);
  if (!o) return undefined;
  const leadName = str(o.leadName);
  const sidekickName = str(o.sidekickName);
  if (!leadName || !sidekickName) return undefined;
  return {
    leadName,
    sidekickName,
    leadEffort: str(o.leadEffort) ?? 'medium',
    sidekickEffort: str(o.sidekickEffort) ?? 'medium',
    ...(str(o.leadKey) ? { leadKey: str(o.leadKey) } : {}),
    ...(str(o.sidekickKey) ? { sidekickKey: str(o.sidekickKey) } : {}),
    ...(typeof o.savedUsd === 'number' && Number.isFinite(o.savedUsd) ? { savedUsd: o.savedUsd } : {}),
    ...(o.busy === true ? { busy: true } : {}),
  };
}

const service = <T,>(s: SessionHandle, name: string, payload?: Record<string, unknown>): Promise<T> =>
  s.cmd<T>({ type: '_oino/service_call', name, ...(payload ? { payload } : {}) });

/** The pair the extension has active + its published status; `undefined` when the service is off. */
export async function fetchFusion(s: SessionHandle): Promise<FusionState | undefined> {
  try {
    const reply = await service<unknown>(s, FUSION_STATUS);
    const o = obj(reply) ?? {};
    return { active: parseFusionActive(o.active), status: parseFusionStatus(o.status) };
  } catch {
    return undefined;
  }
}

/** Whether `unipi.fusion` is installed and enabled for this session (the Fusion tab's gate). */
export async function fusionAvailable(s: SessionHandle): Promise<boolean> {
  try {
    const reply = await s.cmd<unknown>({ type: '_oino/extensions' });
    const o = obj(reply);
    const list = o && Array.isArray(o.extensions) ? o.extensions : Array.isArray(reply) ? reply : [];
    return list.some((ext) => {
      const e = obj(ext);
      return e?.id === FUSION_EXTENSION_ID && e.enabled !== false;
    });
  } catch {
    return false;
  }
}

/** Ask the extension to switch to `active` (the picker's apply). Throws the extension's message. */
export async function setFusionActive(s: SessionHandle, active: FusionActive): Promise<void> {
  const reply = await service<unknown>(s, FUSION_SET_PAIR, { active });
  const o = obj(reply);
  if (o && o.ok === false) throw new Error(str(o.message) ?? 'Fusion did not accept the pair.');
}
