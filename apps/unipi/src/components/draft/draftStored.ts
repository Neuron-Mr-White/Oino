/**
 * The draft's picks persist across route changes (Z-19b): DraftView unmounts
 * when the user navigates away, and the route root disposes the stand-in
 * session — model, thinking level, Fusion pair, permission, autonomy, plan
 * and the workspace were held only in memory, so returning to `/new/<host>`
 * re-seeded them from the global defaults. The composer's TEXT already
 * persists (`lib/composer/drafts.ts`); this makes the toggles persist the
 * same way, per host: `unipi:draft:choices:<hostId>`.
 */
import type { DraftChoices } from './draftSession';

/** What a draft remembers: its choices and the picked workspace folder. */
export type DraftStored = { choices?: DraftChoices; cwd?: string };

const storageKey = (hostId: string) => `unipi:draft:choices:${hostId}`;

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

const str = (v: unknown): string | undefined => (typeof v === 'string' && v !== '' ? v : undefined);

/** A stored model row; half-picked entries are dropped. */
const model = (v: unknown): DraftChoices['model'] => {
  if (!isRecord(v)) return undefined;
  const provider = str(v.provider);
  const modelId = str(v.modelId);
  if (!provider || !modelId) return undefined;
  return { provider, modelId, ...(str(v.name) ? { name: str(v.name) } : {}) };
};

const fusion = (v: unknown): DraftChoices['fusion'] => {
  if (!isRecord(v)) return undefined;
  const lead = str(v.lead);
  const sidekick = str(v.sidekick);
  const leadEffort = str(v.leadEffort);
  const sidekickEffort = str(v.sidekickEffort);
  if (!lead || !sidekick || !leadEffort || !sidekickEffort) return undefined;
  return { lead, sidekick, leadEffort, sidekickEffort };
};

/** A tolerant read: wrong shapes are dropped, not fatal. */
export function loadDraft(hostId: string): DraftStored {
  try {
    const raw = window.localStorage.getItem(storageKey(hostId));
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed)) return {};
    const out: DraftStored = {};
    const cwd = str(parsed.cwd);
    if (cwd !== undefined) out.cwd = cwd;
    if (isRecord(parsed.choices)) {
      const c = parsed.choices;
      const choices: DraftChoices = {};
      const modelRow = model(c.model);
      if (modelRow) choices.model = modelRow;
      const pair = fusion(c.fusion);
      if (pair) choices.fusion = pair;
      if (str(c.thinking)) choices.thinking = str(c.thinking);
      if (str(c.permission)) choices.permission = str(c.permission);
      if (c.autonomy === 'low' || c.autonomy === 'medium' || c.autonomy === 'high') choices.autonomy = c.autonomy;
      if (typeof c.plan === 'boolean') choices.plan = c.plan;
      if (Object.keys(choices).length) out.choices = choices;
    }
    return out;
  } catch {
    return {};
  }
}

/** A fresh read-modify-write of the whole record (one key per host). */
function patch(hostId: string, patchEntry: { choices?: DraftChoices; cwd?: string | null }): void {
  try {
    const key = storageKey(hostId);
    const stored = loadDraft(hostId);
    const next: DraftStored = {
      ...(patchEntry.choices !== undefined ? { choices: patchEntry.choices } : { choices: stored.choices }),
      ...(patchEntry.cwd === null ? {} : patchEntry.cwd !== undefined ? { cwd: patchEntry.cwd } : stored.cwd !== undefined ? { cwd: stored.cwd } : {}),
    };
    if (!next.choices && next.cwd === undefined) {
      window.localStorage.removeItem(key);
      return;
    }
    window.localStorage.setItem(key, JSON.stringify(next));
  } catch {
    // Storage unavailable: the draft stays in memory only.
  }
}

export function saveDraftChoices(hostId: string, choices: DraftChoices): void {
  patch(hostId, { choices: Object.keys(choices).length ? choices : undefined });
}

export function saveDraftCwd(hostId: string, cwd: string | undefined): void {
  // `null` clears a previously stored folder (the "No workspace" pick).
  patch(hostId, { cwd: cwd ?? null });
}

export function clearDraft(hostId: string): void {
  try {
    window.localStorage.removeItem(storageKey(hostId));
  } catch {
    // Storage unavailable: the draft stays in memory only.
  }
}
