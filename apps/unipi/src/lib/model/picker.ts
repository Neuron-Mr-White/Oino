/**
 * Model picker data (WP-A14, Z-69 / Z-76): `get_available_models` returns pi
 * `Model` JSON (`{provider, id, name, input, contextWindow, reasoning, …}`);
 * the sheet's Single list, the Fusion lead/sidekick columns sharing one
 * search, the checked current row and the one explicit apply action are pure
 * rules — the component stays markup.
 *
 * The catalogue is the host's (credentials-configured models first, as core
 * orders it); the app adds no curated list. Fusion's pair comes from
 * `unipi.fusion` (`./fusion.ts`), so the picker never invents one.
 */

/** The part of pi's `Model` the picker reads. */
export type ModelInfo = {
  provider: string;
  id: string;
  name?: string;
  /** The model reasons → it has thinking levels (pi `reasoning`). */
  reasoning?: boolean;
  /** pi `input` modalities (`text`, `image`). */
  input?: string[];
  /** Context window in tokens. */
  contextWindow?: number;
  maxTokens?: number;
};

/** `provider/id` — the key both the host and the state use. */
export const modelKey = (m: { provider?: string; id?: string }): string => `${m.provider ?? ''}/${m.id ?? ''}`;

const str = (v: unknown): string | undefined => (typeof v === 'string' && v ? v : undefined);
const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);

/** `get_available_models` reply (`{models: Model[]}`) → the rows; anything else → none. */
export function parseModels(reply: unknown): ModelInfo[] {
  const list = Array.isArray(reply)
    ? reply
    : reply && typeof reply === 'object'
      ? ((reply as { models?: unknown }).models ?? [])
      : [];
  if (!Array.isArray(list)) return [];
  const out: ModelInfo[] = [];
  const seen = new Set<string>();
  for (const raw of list) {
    if (!raw || typeof raw !== 'object') continue;
    const o = raw as Record<string, unknown>;
    const provider = str(o.provider);
    const id = str(o.id);
    if (!provider || !id) continue;
    const key = `${provider}/${id}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({
      provider,
      id,
      ...(str(o.name) ? { name: str(o.name) } : {}),
      ...(typeof o.reasoning === 'boolean' ? { reasoning: o.reasoning } : {}),
      ...(Array.isArray(o.input) ? { input: o.input.filter((x): x is string => typeof x === 'string') } : {}),
      ...(num(o.contextWindow) !== undefined ? { contextWindow: num(o.contextWindow) } : {}),
      ...(num(o.maxTokens) !== undefined ? { maxTokens: num(o.maxTokens) } : {}),
    });
  }
  return out;
}

/** "Anthropic: Claude Fable 5" → "Claude Fable 5": the provider prefix eats the narrow Fusion columns. */
export function shortModelName(name: string | undefined): string {
  const n = (name ?? '').trim();
  const m = /^[^:]{1,24}:\s+(.+)$/.exec(n);
  return m ? m[1]! : n;
}

/** The row's label: the display name, else the id (snapshots carry no name). */
export const modelName = (m: ModelInfo): string => (m.name && m.name.trim()) || m.id;

/** The Vision badge (Z-69): the model accepts image input. */
export const hasVision = (m: ModelInfo): boolean => (m.input ?? []).includes('image');

/** The context badge (Z-69): `200k` / `1M`; nothing when the catalogue has no window. */
export function contextBadge(m: ModelInfo): string | undefined {
  const n = m.contextWindow;
  if (!n || n <= 0) return undefined;
  if (n >= 1_000_000) return `${Number((n / 1_000_000).toFixed(n % 1_000_000 === 0 ? 0 : 1))}M`;
  return `${Math.round(n / 1000)}k`;
}

/** What the composer/sheet marks as running: one model, or Fusion's pair. */
export type Selection =
  | { kind: 'single'; key: string }
  | { kind: 'fusion'; lead: string; sidekick: string; leadEffort?: string; sidekickEffort?: string };

/**
 * The selection pi is really on (UNI-212): Fusion only while the fusion
 * extension's active pair's lead is still pi's model — a `/model` switch in
 * the TUI must never leave the picker marking Fusion as current.
 */
export function selectionOf(
  model: { provider?: string; id?: string } | undefined,
  fusion:
    | { kind: 'single'; model: string }
    | { kind: 'fusion'; lead: string; sidekick: string; leadEffort?: string; sidekickEffort?: string }
    | undefined,
): Selection | undefined {
  const key = model?.provider && model.id ? modelKey(model) : undefined;
  if (fusion?.kind === 'fusion' && (!key || fusion.lead === key)) return fusion;
  return key ? { kind: 'single', key } : undefined;
}

const hits = (m: ModelInfo, q: string): boolean => `${modelKey(m)} ${m.name ?? ''}`.toLowerCase().includes(q);

/**
 * The Single tab's rows: the running model first, then the catalogue in the
 * host's order; a search spans the whole catalogue (Z-69, QA#4 — one query,
 * every row). At most `limit` rows are rendered by the list.
 */
export function singleModels(all: readonly ModelInfo[], current: Selection | undefined, query: string): ModelInfo[] {
  const q = query.trim().toLowerCase();
  if (q) return all.filter((m) => hits(m, q));
  if (!current || current.kind !== 'single') return [...all];
  const at = all.findIndex((m) => modelKey(m) === current.key);
  if (at <= 0) return [...all];
  const first = all[at]!;
  return [first, ...all.slice(0, at), ...all.slice(at + 1)];
}

/** A Fusion column's rows: the same shared search filters both columns (Z-76, QA#4). */
export function columnModels(all: readonly ModelInfo[], query: string): ModelInfo[] {
  const q = query.trim().toLowerCase();
  return q ? all.filter((m) => hits(m, q)) : [...all];
}

/** Fusion effort pills (unipi preset levels). */
export const FUSION_EFFORTS = ['low', 'medium', 'high'] as const;

/** The Single tab's one explicit action (UNI-212): nothing when the tapped row changes nothing. */
export function singleAction(current: Selection | undefined, pending: string | undefined): { label: string; offFusion: boolean } | undefined {
  if (!pending) return undefined;
  if (current?.kind === 'single' && current.key === pending) return undefined;
  return current?.kind === 'fusion' ? { label: 'Turn Fusion off & switch', offFusion: true } : { label: 'Switch model', offFusion: false };
}

/** Next pending Single pick: tapping the same row again, or the running model, means "no change". */
export function nextSinglePick(current: Selection | undefined, pending: string | undefined, key: string): string | undefined {
  if (pending === key) return undefined;
  if (current?.kind === 'single' && current.key === key) return undefined;
  return key;
}

/** Is the candidate Fusion pair the one already running? (the footer's disabled "Current pair"). */
export function pairUnchanged(
  current: Selection | undefined,
  lead: string | undefined,
  sidekick: string | undefined,
  leadEffort: string,
  sidekickEffort: string,
): boolean {
  return (
    !!current &&
    current.kind === 'fusion' &&
    current.lead === lead &&
    current.sidekick === sidekick &&
    (current.leadEffort ?? 'medium') === leadEffort &&
    (current.sidekickEffort ?? 'medium') === sidekickEffort
  );
}

/** The sheet's "Now …" line (Z-69, sentence case): what runs right now. */
export function nowLabel(current: Selection | undefined, nameOf: (key: string) => string): string {
  if (!current) return 'No model yet';
  if (current.kind === 'fusion') return `Fusion · ${shortModelName(nameOf(current.lead))} ◆ ${shortModelName(nameOf(current.sidekick))}`;
  return shortModelName(nameOf(current.key)) || current.key;
}

/** The switch toast (Z-72): "Switched from X to Y". */
export function switchToast(from: string | undefined, to: string): string {
  return from && from !== to ? `Switched from ${from} to ${to}` : `Switched to ${to}`;
}

/**
 * The Fusion apply toast: another pair over the running one is a pair update
 * ("Fusion pair: A ◆ B"), otherwise it is the plain X → Y switch.
 */
export function pairToast(from: string, to: string): string {
  const bare = to.replace(/^Fusion · /, '');
  return from.startsWith('Fusion') ? `Fusion pair: ${bare}` : switchToast(from, to);
}
