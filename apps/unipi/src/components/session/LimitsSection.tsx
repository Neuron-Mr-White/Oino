/**
 * Limits section (WP-A17, G-16): this session's watchdog caps — max turns,
 * max cost (USD), max wall minutes, max consecutive tool errors — plus the
 * summary-turn switch. Data and writes go through the `unipi.watchdog`
 * services (`_oino/service_call`): `caps.get` → `{caps, pinned}`,
 * `caps.set` pins for this session and persists the pins as a
 * `watchdog.session-caps` custom entry. The section hides when the service
 * is not there (older host / watchdog off) — no dead UI.
 */
import { createEffect, createResource, createSignal, Show } from 'solid-js';
import { type SessionHandle } from '../../lib/session/store';
import { serviceCall, liveSource } from './modalData';

export type Caps = {
  maxTurns: number | null;
  maxCostUsd: number | null;
  maxWallMinutes: number | null;
  maxConsecutiveToolErrors: number | null;
  summaryTurn: boolean;
};

export type CapsReply = { caps: Caps; pinned: boolean } | undefined;

export const LIMIT_FIELDS: Array<{ key: keyof Omit<Caps, 'summaryTurn'>; label: string; placeholder: string }> = [
  { key: 'maxTurns', label: 'Max turns', placeholder: 'e.g. 40' },
  { key: 'maxCostUsd', label: 'Max cost (USD)', placeholder: 'e.g. 2.50' },
  { key: 'maxWallMinutes', label: 'Max wall (min)', placeholder: 'e.g. 30' },
  { key: 'maxConsecutiveToolErrors', label: 'Max tool errors', placeholder: 'e.g. 5' },
];

const parseCaps = (v: unknown): Caps => {
  const o = (v && typeof v === 'object' ? v : {}) as Record<string, unknown>;
  const n = (x: unknown): number | null => (typeof x === 'number' && Number.isFinite(x) && x > 0 ? x : null);
  return {
    maxTurns: n(o.maxTurns),
    maxCostUsd: n(o.maxCostUsd),
    maxWallMinutes: n(o.maxWallMinutes),
    maxConsecutiveToolErrors: n(o.maxConsecutiveToolErrors),
    summaryTurn: o.summaryTurn !== false,
  };
};

export async function fetchCaps(s: SessionHandle): Promise<CapsReply> {
  let reply: unknown;
  try {
    reply = await serviceCall<unknown>(s, 'caps.get', undefined, 'unipi.watchdog');
  } catch {
    // No `caps.get` on this host (watchdog off / older host): the section hides.
    return undefined;
  }
  if (!reply || typeof reply !== 'object') return undefined;
  const o = reply as Record<string, unknown>;
  return { caps: parseCaps(o.caps), pinned: o.pinned === true };
}

/** The `caps.set` payload from the form: empty fields pin nothing (`null`). */
export function pinsOf(form: Record<string, string>, summaryTurn: boolean): Record<string, unknown> {
  const pins: Record<string, unknown> = { summaryTurn };
  for (const f of LIMIT_FIELDS) {
    const raw = (form[f.key] ?? '').trim();
    const n = Number(raw);
    pins[f.key] = raw !== '' && Number.isFinite(n) && n > 0 ? n : null;
  }
  return pins;
}

export function LimitsSection(props: { s: SessionHandle }) {
  const [data, { refetch }] = createResource(liveSource(() => props.s), async (state) => (state === 'live' ? fetchCaps(props.s) : undefined));
  const [form, setForm] = createSignal<Record<string, string>>({});
  const [summary, setSummary] = createSignal(true);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | undefined>();
  // The form always starts from the session's effective caps.
  let seeded = false;
  const caps = () => data.latest?.caps;
  createEffect(() => {
    if (seeded || !caps()) return;
    seeded = true;
    const next: Record<string, string> = {};
    for (const f of LIMIT_FIELDS) next[f.key] = caps()![f.key] == null ? '' : String(caps()![f.key]);
    setForm(next);
    setSummary(caps()!.summaryTurn);
  });

  const save = async () => {
    if (busy()) return;
    setBusy(true);
    setError(undefined);
    try {
      await serviceCall(props.s, 'caps.set', pinsOf(form(), summary()), 'unipi.watchdog');
      await refetch();
      seeded = false;
      if (caps()) {
        const next: Record<string, string> = {};
        for (const f of LIMIT_FIELDS) next[f.key] = caps()![f.key] == null ? '' : String(caps()![f.key]);
        setForm(next);
        setSummary(caps()!.summaryTurn);
        seeded = true;
      }
    } catch (e) {
      setError(String((e as { message?: string })?.message ?? e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Show when={data.latest !== undefined && caps()}>
      <div class="sm-limits" data-testid="sm-limits">
        <div class="sm-section-title" style={{ margin: 0 }}>
          Limits
          <span class="sm-grow" />
          <Show when={data.latest?.pinned}>
            <span class="sm-tree-count">set for this session</span>
          </Show>
        </div>
        <div class="sm-limits-grid">
          {LIMIT_FIELDS.map((f) => (
            <div class="sm-limit">
              <label for={`sm-limit-${f.key}`}>{f.label}</label>
              <input
                id={`sm-limit-${f.key}`}
                type="number"
                min="1"
                step="any"
                inputmode="decimal"
                placeholder={f.placeholder}
                value={form()[f.key] ?? ''}
                onInput={(e) => setForm((cur) => ({ ...cur, [f.key]: e.currentTarget.value }))}
              />
            </div>
          ))}
        </div>
        <label class="sm-switch-line">
          <input type="checkbox" checked={summary()} onChange={(e) => setSummary(e.currentTarget.checked)} />
          Ask for a summary when a limit stops the run
        </label>
        <div class="sm-limits-foot">
          <span class="sm-status" data-error={error() ? '' : undefined}>
            {error() ?? 'A run aborts at the first limit reached. Empty means off.'}
          </span>
          <button type="button" class="btn btn-primary btn-sm" disabled={busy()} aria-busy={busy() ? 'true' : undefined} onClick={() => void save()} data-testid="sm-limits-save">
            {busy() ? 'Saving…' : 'Save limits'}
          </button>
        </div>
      </div>
    </Show>
  );
}
