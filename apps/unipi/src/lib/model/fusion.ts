/**
 * Fusion pairing data (WP-A14, Z-76; extension `unipi.fusion`): the one
 * surface the app reads the lead/sidekick pair from and sets it through —
 * services `fusion.status` and `fusion.set-pair` (`_oino/service_call`).
 * The app holds no Fusion state of its own: the extension's `Active` record
 * is the truth, the picker only shows it and asks for a change.
 */
import { createSignal } from 'solid-js';
import type { SessionHandle } from '../session/store';

/** Extension id gating the Fusion tab. */
export const FUSION_EXTENSION_ID = 'unipi.fusion';
/** Sticky bus topic published by `unipi.fusion`. */
export const FUSION_STATUS_TOPIC = 'unipi:fusion:status';
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

/** One curated entry (`{key, name}`; the name resolved by the extension, id fallback). */
export type CuratedEntry = { key: string; name: string };

/**
 * `curated` of `fusion.status` (P0-10): the lists `/fusion-preset` keeps.
 * `ranked` = recent → curated leads → sidekicks, deduped, catalogue-validated
 * (the TUI picker's head order); `effort` = the per-model default effort.
 */
export type FusionCurated = {
  lead: CuratedEntry[];
  sidekick: CuratedEntry[];
  ranked: CuratedEntry[];
  default: { lead?: string; sidekick?: string };
  effort: Record<string, string>;
};

/** `{active, status, curated}` from `fusion.status`. */
export type FusionState = { active?: FusionActive; status?: FusionStatus; curated?: FusionCurated };

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

const entries = (v: unknown): CuratedEntry[] => {
  if (!Array.isArray(v)) return [];
  const out: CuratedEntry[] = [];
  const seen = new Set<string>();
  for (const raw of v) {
    const o = obj(raw);
    const key = str(o?.key) ?? str(raw);
    if (!key || seen.has(key)) continue;
    seen.add(key);
    out.push({ key, name: str(o?.name) ?? key.slice(key.indexOf('/') + 1) });
  }
  return out;
};

/** The `curated` payload, leniently (`undefined` when absent — an older extension). */
export function parseFusionCurated(raw: unknown): FusionCurated | undefined {
  const o = obj(raw);
  if (!o) return undefined;
  const def = obj(o.default) ?? {};
  const effort: Record<string, string> = {};
  for (const [k, v] of Object.entries(obj(o.effort) ?? {})) if (str(v)) effort[k] = v as string;
  return {
    lead: entries(o.lead),
    sidekick: entries(o.sidekick),
    ranked: entries(o.ranked),
    default: { ...(str(def.lead) ? { lead: str(def.lead) } : {}), ...(str(def.sidekick) ? { sidekick: str(def.sidekick) } : {}) },
    effort,
  };
}

const service = <T,>(s: SessionHandle, name: string, payload?: Record<string, unknown>): Promise<T> =>
  s.cmd<T>({ type: '_oino/service_call', name, ...(payload ? { payload } : {}) });

/** Reactive per-session cache updated immediately when `setFusionActive` or `fetchFusion` runs. */
const [fusionOverrides, setFusionOverrides] = createSignal<Record<string, { active: FusionActive | null; status?: FusionStatus }>>({});

export function resetFusionOverrides(): void {
  setFusionOverrides({});
}

/** Restores active Fusion pair from persisted `unipi:fusion-sidekick` session entries (cold snapshot fallback). */
export function fusionFromEntries(entries: ReadonlyArray<{ type?: string; customType?: string; data?: unknown }>): FusionActive | null | undefined {
  for (let i = entries.length - 1; i >= 0; i--) {
    const e = entries[i]!;
    if (e.customType !== 'unipi:fusion-sidekick') continue;
    const d = obj(e.data);
    if (!d) return null;
    if (d.active !== true) return null;
    const parsed = parseFusionActive(d.selection);
    return parsed?.kind === 'fusion' ? parsed : null;
  }
  return undefined;
}

/** Active Fusion pair for a session (topic, optimistic override, or persisted entry), gated on matching lead model. */
export function activeSessionFusion(s: Pick<SessionHandle, 'key' | 'state'>): {
  lead: string;
  sidekick: string;
  leadName?: string;
  sidekickName?: string;
  leadEffort?: string;
  sidekickEffort?: string;
} | undefined {
  const model = s.state.model;
  const curKey = model?.provider && model.id ? `${model.provider}/${model.id}` : undefined;
  const topicRaw = s.state.topics?.[FUSION_STATUS_TOPIC];
  const topicSt = topicRaw ? parseFusionStatus(topicRaw.data) : undefined;
  if (topicRaw && topicRaw.data === null) return undefined;
  if (topicSt && topicSt.leadKey && topicSt.sidekickKey) {
    if (curKey && topicSt.leadKey !== curKey) return undefined;
    return {
      lead: topicSt.leadKey,
      sidekick: topicSt.sidekickKey,
      leadName: topicSt.leadName,
      sidekickName: topicSt.sidekickName,
      leadEffort: topicSt.leadEffort,
      sidekickEffort: topicSt.sidekickEffort,
    };
  }
  const entry = fusionOverrides()[s.key];
  const over = entry?.active;
  const st = topicSt ?? entry?.status;
  if (over === null || over?.kind === 'single') return undefined;
  const cand = over?.kind === 'fusion' ? over : fusionFromEntries(s.state.entries ?? []);
  if (!cand || cand.kind !== 'fusion') return undefined;
  if (curKey && cand.lead !== curKey) return undefined;
  const sameCached = entry?.active?.kind === 'fusion' && entry.active.lead === cand.lead && entry.active.sidekick === cand.sidekick;
  return {
    lead: cand.lead,
    sidekick: cand.sidekick,
    ...((topicSt || sameCached) && st?.leadName ? { leadName: st.leadName } : {}),
    ...((topicSt || sameCached) && st?.sidekickName ? { sidekickName: st.sidekickName } : {}),
    leadEffort: cand.leadEffort ?? st?.leadEffort,
    sidekickEffort: cand.sidekickEffort ?? st?.sidekickEffort,
  };
}

/** The pair the extension has active + its published status; `undefined` when the service is off. */
export async function fetchFusion(s: SessionHandle): Promise<FusionState | undefined> {
  try {
    const reply = await service<unknown>(s, FUSION_STATUS);
    const o = obj(reply) ?? {};
    const curated = parseFusionCurated(o.curated);
    const active = parseFusionActive(o.active);
    const status = parseFusionStatus(o.status);
    setFusionOverrides((prev) => ({ ...prev, [s.key]: { active: active ?? null, ...(status ? { status } : {}) } }));
    return { active, status, ...(curated ? { curated } : {}) };
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
  setFusionOverrides((prev) => ({ ...prev, [s.key]: { active } }));
}
