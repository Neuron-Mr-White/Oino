/**
 * Session control centre (UNI-160, split in UNI-201): the chat title and
 * the composer's model chip open a Model-only sheet (`ModelPickerSheet`:
 * single/Fusion switch, thinking level, the model list — no tabs); the
 * header ⋯ menu and tray rows open the Session sheet (`ControlCentreSheet`:
 * Context / Running / Info tabs). Keeping the two apart is what makes the
 * subheader tray short — it used to dump the whole Running list on screen.
 */
import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from 'solid-js';
import { Brain, Check, ChevronRight, Cpu, Gauge, ListChecks, Info as InfoIcon, X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import type { Chat, WorkItem } from '../../lib/chat/store';
import { formatDuration } from '../../lib/chat/model';
import { desktopShell } from '../../lib/layout';

/** Session sheet tabs (UNI-201: `model` moved to its own `ModelPickerSheet`). */
type Section = 'context' | 'running' | 'info';

const SECTIONS: Array<{ id: Section; label: string; icon: typeof Cpu }> = [
  { id: 'context', label: 'Context', icon: Gauge },
  { id: 'running', label: 'Running', icon: ListChecks },
  { id: 'info', label: 'Info', icon: InfoIcon },
];

export const DOT_LABEL: Record<WorkItem['dot'], string> = { running: 'Running', stopped: 'Stopped', failed: 'Failed', done: 'Done' };



type PickerModel = { provider: string; id: string; name?: string; reasoning?: boolean };
/** The Single list: current + curated when not searching; search spans everything. */
export function pickerModels(
  all: readonly PickerModel[],
  curated: readonly { key: string }[] | undefined,
  current: { provider: string; id: string } | undefined,
  query: string,
): PickerModel[] {
  const key = (m: { provider: string; id: string }) => `${m.provider}/${m.id}`;
  const byKey = new Map(all.map((m) => [key(m), m]));
  const q = query.trim().toLowerCase();
  if (q) {
    const hit = (m: PickerModel) => `${key(m)} ${m.name ?? ''}`.toLowerCase().includes(q);
    const curatedKeys = new Set((curated ?? []).map((c) => c.key));
    const matches = all.filter(hit);
    return [...matches.filter((m) => curatedKeys.has(key(m))), ...matches.filter((m) => !curatedKeys.has(key(m)))];
  }
  const out: PickerModel[] = [];
  const seen = new Set<string>();
  const add = (m: PickerModel | undefined) => {
    if (m && !seen.has(key(m))) {
      seen.add(key(m));
      out.push(m);
    }
  };
  if (current) add(byKey.get(key(current)) ?? (current as PickerModel));
  for (const c of curated ?? []) add(byKey.get(c.key));
  // Nothing curated (no Fusion preset): the bridge already orders the list
  // (scoped, configured providers first) — show its head.
  if (out.length <= 1) for (const m of all.slice(0, 40)) add(m);
  return out;
}
/** "Anthropic: Claude Fable 5" → "Claude Fable 5": the provider prefix eats the narrow Fusion columns. */
export function shortModelName(name: string | undefined): string {
  const n = (name ?? '').trim();
  const m = /^[^:]{1,24}:\s+(.+)$/.exec(n);
  return m ? m[1]! : n;
}
/** `$1.23` / `$0.004` — small costs keep more decimals so they don't all read as $0.00. */
export function formatUsd(n: number | undefined): string {
  if (!n) return '$0.00';
  return n < 0.01 ? `$${n.toFixed(4)}` : `$${n.toFixed(2)}`;
}

/** `1.2k` / `845` — compact token counts for the context section. */
export function formatCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

/** `Fusion · lead ◆ sidekick` when Fusion is active, else the single model's name — the chat header subtitle. */
export function modelHeaderLabel(chat: Chat): string {
  const s = chat.state;
  const fusion = s.fusionStatus;
  if (fusion) return `Fusion · ${fusion.leadName || '…'} ◆ ${fusion.sidekickName || '…'}`;
  return s.run.model?.name ?? s.run.model?.id ?? '';
}

/** `mode · model · ctx % · cost` footer strip, one compact line. */
export function footerStripText(chat: Chat): string {
  const s = chat.state;
  const fusion = s.fusionStatus;
  const mode = fusion ? 'Fusion' : 'Single';
  const model = fusion ? fusion.leadName || '…' : s.run.model?.name ?? s.run.model?.id ?? '—';
  const pct = s.run.context?.percent;
  const ctx = pct === null || pct === undefined ? '—' : `${Math.round(pct)}%`;
  const cost = formatUsd(s.stats?.cost ?? s.run.cost);
  return `${mode} · ${model} · ctx ${ctx} · ${cost}`;
}

function ContextSection(props: { chat: Chat }) {
  const s = () => props.chat.state;
  createEffect(() => {
    props.chat.watch({ stats: true });
    onCleanup(() => props.chat.watch({ stats: false }));
  });
  const ctx = () => s().run.context;
  const stats = () => s().stats;
  return (
    <div class="cc-context">
      <Show when={ctx()}>
        {(c) => (
          <div class="cc-meter">
            <div class="cc-meter-bar">
              <div class="cc-meter-fill" data-high={(c().percent ?? 0) >= 80 ? '' : undefined} style={{ width: `${Math.min(100, Math.max(0, c().percent ?? 0))}%` }} />
            </div>
            <span class="cc-meter-label">
              {c().percent === null ? '—' : `${Math.round(c().percent!)}%`} of context · {c().tokens ?? '—'} / {c().window} tokens
            </span>
          </div>
        )}
      </Show>
      <div class="cc-stat-grid">
        <div class="cc-stat">
          <span class="cc-stat-label">Tokens in</span>
          <span class="cc-stat-value">{formatCount(stats()?.tokensIn ?? 0)}</span>
        </div>
        <div class="cc-stat">
          <span class="cc-stat-label">Tokens out</span>
          <span class="cc-stat-value">{formatCount(stats()?.tokensOut ?? 0)}</span>
        </div>
        <div class="cc-stat">
          <span class="cc-stat-label">Cache hit</span>
          <span class="cc-stat-value">{formatCount(stats()?.cacheHit ?? 0)}</span>
        </div>
        <div class="cc-stat">
          <span class="cc-stat-label">tok/s</span>
          <span class="cc-stat-value">{Math.round(stats()?.tps ?? 0)}</span>
        </div>
      </div>
      <p class="cc-cost">Session cost: {formatUsd(stats()?.cost ?? s().run.cost)}</p>
      <Show when={s().fusionStatus?.savedUsd}>
        <p class="cc-cost cc-cost-saved">Fusion saved ~{formatUsd(s().fusionStatus?.savedUsd)}</p>
      </Show>
    </div>
  );
}

export function WorkRow(props: { item: WorkItem; onOpen: () => void }) {
  return (
    <button type="button" class="model-row cc-work-row" onClick={props.onOpen}>
      <span class={`cc-dot cc-dot-${props.item.dot}`} aria-hidden="true" />
      <span class="min-w-0 flex-1 text-left">
        <span class="block truncate">{props.item.title}</span>
        <span class="block truncate model-row-sub">
          {DOT_LABEL[props.item.dot]}
          {props.item.detail ? ` · ${props.item.detail}` : ''}
        </span>
      </span>
      <ChevronRight size={16} class="shrink-0 board-accent" style={{ opacity: 0.4 }} />
    </button>
  );
}

function WorkDetail(props: { chat: Chat; item: WorkItem; onClose: () => void; onToast: (t: string) => void }) {
  const [log, setLog] = createSignal<string>('');
  const [loading, setLoading] = createSignal(false);
  const [confirmStop, setConfirmStop] = createSignal(false);

  createEffect(() => {
    if (props.item.kind !== 'bg') return;
    setLoading(true);
    void props.chat.workLog(props.item.id).then((page) => {
      setLoading(false);
      setLog('error' in page ? page.error : page.text);
    });
  });

  const stop = () => {
    void props.chat
      .workStop(props.item.id)
      .then(() => props.onClose())
      .catch((e: unknown) => props.onToast(String((e as { message?: string })?.message ?? e)));
  };
  const background = () => {
    void props.chat
      .workBackground(props.item.id)
      .then(() => props.onClose())
      .catch((e: unknown) => props.onToast(String((e as { message?: string })?.message ?? e)));
  };
  const rerun = () => {
    void props.chat
      .workRerun(props.item.id)
      .then(() => props.onClose())
      .catch((e: unknown) => props.onToast(String((e as { message?: string })?.message ?? e)));
  };

  return (
    <div class="cc-work-detail">
      <div class="cc-work-detail-head">
        <span class={`cc-dot cc-dot-${props.item.dot}`} aria-hidden="true" />
        <span class="min-w-0 flex-1 truncate font-semibold">{props.item.title}</span>
        <button type="button" class="ghost-icon-button" aria-label="Back" onClick={props.onClose}>
          <X size={18} />
        </button>
      </div>
      <Show when={props.item.kind === 'bg'}>
        <pre class="cc-work-log">{loading() ? 'Loading…' : log() || '(no output yet)'}</pre>
      </Show>
      <Show when={props.item.kind === 'subagent'}>
        <p class="move-why">{props.item.detail || 'No report yet.'}</p>
      </Show>
      <Show when={props.item.kind === 'sidekick'}>
        <p class="move-why">{props.item.detail}</p>
      </Show>
      <Show when={!confirmStop()} fallback={
        <div class="flex gap-3 mt-2">
          <button type="button" class="pair-secondary flex-1" onClick={() => setConfirmStop(false)}>
            Cancel
          </button>
          <button type="button" class="pair-primary flex-[1.4]" onClick={stop}>
            Stop it
          </button>
        </div>
      }>
        <div class="cc-work-actions">
          <Show when={props.item.canStop}>
            <button type="button" class="queue-action-btn" data-tone="danger" onClick={() => setConfirmStop(true)}>
              Stop
            </button>
          </Show>
          <Show when={props.item.canRerun}>
            <button type="button" class="queue-action-btn" onClick={rerun}>
              Re-run
            </button>
          </Show>
          <Show when={props.item.canBackground}>
            <button type="button" class="queue-action-btn" onClick={background}>
              Send to background
            </button>
          </Show>
        </div>
      </Show>
    </div>
  );
}

function RunningSection(props: { chat: Chat; onToast: (t: string) => void }) {
  const [detail, setDetail] = createSignal<WorkItem | null>(null);
  const items = () => props.chat.state.work;
  createEffect(() => {
    const id = detail()?.id;
    if (!id) return;
    const fresh = items().find((i) => i.id === id);
    if (fresh) setDetail(fresh);
  });
  return (
    <Show when={!detail()} fallback={<WorkDetail chat={props.chat} item={detail()!} onClose={() => setDetail(null)} onToast={props.onToast} />}>
      <Show when={items().length > 0} fallback={<p class="board-menu-note">Nothing running right now.</p>}>
        <div class="model-list">
          <For each={items()}>{(item) => <WorkRow item={item} onOpen={() => setDetail(item)} />}</For>
        </div>
      </Show>
    </Show>
  );
}

function InfoSection(props: { chat: Chat }) {
  createEffect(() => {
    props.chat.watch({ info: true });
    onCleanup(() => props.chat.watch({ info: false }));
  });
  const groups = () => props.chat.state.infoGroups;
  return (
    <Show when={groups().length > 0} fallback={<p class="board-menu-note">No info available.</p>}>
      <div class="cc-info-grid">
        <For each={groups()}>
          {(g) => (
            <div class="cc-info-card">
              <p class="cc-info-card-title">{g.label}</p>
              <For each={g.stats}>
                {(s) => (
                  <div class="cc-info-stat">
                    <span class="cc-info-stat-label">{s.label}</span>
                    <span class="cc-info-stat-value">{s.value}</span>
                  </div>
                )}
              </For>
            </div>
          )}
        </For>
      </div>
    </Show>
  );
}

/** Count of `work` items still running (UNI-188): the subheader badge + header ⋯ dot. */
export function runningWorkCount(chat: Chat): number {
  return chat.state.work.filter((w) => w.dot === 'running').length;
}

/**
 * The subheader strip (UNI-188): the compact `footerStripText` line, an
 * animated "N running" badge while background work is live, and a tap
 * toggles the `FooterTray` below it (no modal).
 */
export function FooterStrip(props: { chat: Chat; open: boolean; onToggle: () => void }) {
  const count = () => runningWorkCount(props.chat);
  return (
    <button type="button" class="chat-footer-strip" aria-expanded={props.open} aria-label="Session details" onClick={props.onToggle}>
      <span class="truncate">{footerStripText(props.chat)}</span>
      <Show when={count() > 0}>
        <span class="footer-strip-badge" data-testid="footer-strip-badge">
          {count()} running
        </span>
      </Show>
    </button>
  );
}

/** "N running · M done" (UNI-201): the tray's single Running row, never
 * the full item list (that lives behind the Running section). */
export function runningSummaryText(chat: Chat): string | undefined {
  const items = chat.state.work;
  if (items.length === 0) return undefined;
  const running = items.filter((w) => w.dot === 'running').length;
  const done = items.length - running;
  const parts = [running > 0 ? `${running} running` : undefined, done > 0 ? `${done} done` : undefined];
  return parts.filter(Boolean).join(' · ') || `${items.length} item${items.length === 1 ? '' : 's'}`;
}

/**
 * The subheader's expand-in-place tray (UNI-188, trimmed in UNI-201): at
 * most four short rows — model+thinking, context % with tokens, cost, and
 * one "Running: N running · M done" row (never the item list, which used
 * to fill the screen) — plus `max-height`/`overflow` in CSS as a safety
 * net. No sheet/overlay: a plain block that animates open below the
 * header. `onOpenModel` opens the Model picker sheet; `onOpenSection`
 * opens the Session sheet at that tab.
 */
export function FooterTray(props: { chat: Chat; open: boolean; onOpenModel: () => void; onOpenSection: (section: Section) => void }) {
  const s = () => props.chat.state;
  const fusion = () => s().fusionStatus;
  const ctx = () => s().run.context;
  const running = () => runningSummaryText(props.chat);
  return (
    <div class="chat-footer-tray" data-open={props.open ? '' : undefined} aria-hidden={!props.open} data-testid="chat-footer-tray">
      <div class="chat-footer-tray-inner">
        <button type="button" class="footer-tray-row" onClick={props.onOpenModel}>
          <span class="footer-tray-label">Model</span>
          <span class="footer-tray-value truncate">
            {fusion() ? `${fusion()!.leadName || '…'} ◆ ${fusion()!.sidekickName || '…'}` : s().run.model?.name ?? s().run.model?.id ?? '—'}
            <Show when={s().run.thinking}> · {s().run.thinking}</Show>
          </span>
        </button>
        <button type="button" class="footer-tray-row" onClick={() => props.onOpenSection('context')}>
          <span class="footer-tray-label">Context</span>
          <span class="footer-tray-value">
            <Show when={ctx()} fallback="—">
              {(c) => (c().percent === null ? '—' : `${Math.round(c().percent!)}% · ${c().tokens ?? '—'}/${c().window}`)}
            </Show>
          </span>
        </button>
        <button type="button" class="footer-tray-row" onClick={() => props.onOpenSection('context')}>
          <span class="footer-tray-label">Cost</span>
          <span class="footer-tray-value">{formatUsd(s().stats?.cost ?? s().run.cost)}</span>
        </button>
        <Show when={running()}>
          <button type="button" class="footer-tray-row" onClick={() => props.onOpenSection('running')}>
            <span class="footer-tray-label">Running</span>
            <span class="footer-tray-value">{running()}</span>
          </button>
        </Show>
        <Show when={s().run.waiting}>
          <p class="footer-tray-waiting">Working… · {s().run.waiting}</p>
        </Show>
      </div>
    </div>
  );
}

/** What pi really runs right now (UNI-212): Fusion only when the preset's
 * `active` says so AND pi's model is still its lead — a /model switch in the
 * TUI (or an older bridge that didn't push the fresh preset) must never
 * leave the picker marking Fusion as current. */
export function currentSelection(s: Pick<Chat['state'], 'fusion' | 'run'>):
  | { kind: 'fusion'; lead: string; sidekick: string; leadEffort?: string; sidekickEffort?: string }
  | { kind: 'single'; key: string | undefined } {
  const a = s.fusion?.active;
  const model = s.run.model ? `${s.run.model.provider}/${s.run.model.id}` : undefined;
  if (a?.kind === 'fusion' && (!model || model === a.lead)) return a;
  return { kind: 'single', key: model };
}

/** The Single tab's footer action (UNI-212): exactly one explicit button
 * when there is something to apply, none otherwise — no phantom "Turn
 * Fusion off" that does nothing. `pending` is the tapped (not yet applied)
 * model; tapping the current model or the pending one again clears it. */
export function singleAction(
  current: ReturnType<typeof currentSelection>,
  pending: string | undefined,
): { label: string; offFusion: boolean } | undefined {
  if (!pending) return undefined;
  if (current.kind === 'single' && current.key === pending) return undefined;
  return current.kind === 'fusion' ? { label: 'Turn Fusion off & switch', offFusion: true } : { label: 'Switch model', offFusion: false };
}

/** Next pending Single pick after tapping `key`: the same row again, or the
 * model already running, means "no change". */
export function nextSinglePick(current: ReturnType<typeof currentSelection>, pending: string | undefined, key: string): string | undefined {
  if (pending === key) return undefined;
  if (current.kind === 'single' && current.key === key) return undefined;
  return key;
}

const errText = (e: unknown) => String((e as { message?: string })?.message ?? e);

/** The Model picker (UNI-201, reworked UNI-212): single/Fusion tabs,
 * thinking level, the model list in a short scroller, and a sticky footer
 * that always holds the one explicit action (Switch model / Use this pair)
 * with its pending → close / inline-error states. Opened by the composer
 * chip, /model, /thinking and the tray's Model row. */
export function ModelPickerSheet(props: { open: boolean; onClose: () => void; chat: Chat; onHaptic: () => void }) {
  const s = () => props.chat.state;
  const current = createMemo(() => currentSelection(s()));
  const fusionOn = () => current().kind === 'fusion';
  const [mode, setMode] = createSignal<'single' | 'fusion'>('single');
  const [q, setQ] = createSignal('');
  const [pick, setPick] = createSignal<string | undefined>(undefined);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  // Fusion pending pair (UNI-176): taps move the local highlight only; the
  // footer button applies it.
  const fusionDefault = () => s().fusion?.default ?? {};
  const activeFusion = () => {
    const c = current();
    return c.kind === 'fusion' ? c : undefined;
  };
  const [pendingLead, setPendingLead] = createSignal<string | undefined>(undefined);
  const [pendingSidekick, setPendingSidekick] = createSignal<string | undefined>(undefined);
  const [leadEffort, setLeadEffort] = createSignal('medium');
  const [sidekickEffort, setSidekickEffort] = createSignal('medium');
  const leadKey = () => pendingLead() ?? activeFusion()?.lead ?? fusionDefault().lead ?? s().fusion?.leads[0]?.key;
  const sidekickKey = () => pendingSidekick() ?? activeFusion()?.sidekick ?? fusionDefault().sidekick ?? s().fusion?.sidekicks[0]?.key;

  // Every open starts from what pi really runs: the matching tab, no
  // leftover pending picks or errors.
  const reset = () => {
    setMode(fusionOn() ? 'fusion' : 'single');
    setQ('');
    setPick(undefined);
    setPendingLead(undefined);
    setPendingSidekick(undefined);
    setError(null);
    setBusy(false);
    const a = activeFusion();
    setLeadEffort(a?.leadEffort ?? 'medium');
    setSidekickEffort(a?.sidekickEffort ?? 'medium');
  };
  createEffect(on(() => props.open, (open) => open && reset()));
  // pi changed under us (TUI switch, another phone): follow it unless the
  // user is mid-apply.
  createEffect(on(() => JSON.stringify(current()), () => !busy() && props.open && reset(), { defer: true }));

  const models = createMemo(() => pickerModels(s().models, s().fusion?.curated, s().run.model, q()));
  const action = () => singleAction(current(), pick());
  const pairUnchanged = () => {
    const a = activeFusion();
    return !!a && a.lead === leadKey() && a.sidekick === sidekickKey() && (a.leadEffort ?? 'medium') === leadEffort() && (a.sidekickEffort ?? 'medium') === sidekickEffort();
  };

  const run = async (op: () => Promise<void>) => {
    if (busy()) return;
    props.onHaptic();
    setBusy(true);
    setError(null);
    try {
      await op();
      setBusy(false);
      props.onClose();
    } catch (e: unknown) {
      setBusy(false);
      // Keep the pending picks so the user can retry or adjust.
      setError(errText(e));
    }
  };
  const applySingle = () => {
    const key = pick();
    if (!key) return;
    const effort = s().run.thinking ?? 'medium';
    void run(() => {
      if (s().fusion) return props.chat.setFusionSingle(key, effort);
      const slash = key.indexOf('/');
      return props.chat.selectModel(key.slice(0, slash), key.slice(slash + 1));
    });
  };
  const applyPair = () => {
    const lead = leadKey();
    const sidekick = sidekickKey();
    if (!lead || !sidekick) return;
    void run(() => props.chat.setFusionPair(lead, sidekick, leadEffort(), sidekickEffort()));
  };
  const setThinking = (level: string) => {
    props.onHaptic();
    setError(null);
    void props.chat.selectThinking(level).catch((e: unknown) => setError(errText(e)));
  };
  const nameOf = (key: string | undefined) => {
    if (!key) return '—';
    const m = s().models.find((x) => `${x.provider}/${x.id}` === key);
    return m?.name ?? s().fusion?.curated?.find((x) => x.key === key)?.name ?? key;
  };
  const currentLabel = () => {
    const c = current();
    if (c.kind === 'fusion') return `Fusion · ${shortModelName(nameOf(c.lead))} ◆ ${shortModelName(nameOf(c.sidekick))}`;
    return s().run.model?.name ?? s().run.model?.id ?? '—';
  };

  const footer = (
    <div class="mp-foot" data-testid="model-picker-foot">
      <Show when={error()}>
        <p class="mp-error" role="alert">
          {error()}
        </p>
      </Show>
      <Show when={mode() === 'fusion' && s().fusion}>
        <button type="button" class="pair-primary" disabled={busy() || !leadKey() || !sidekickKey() || pairUnchanged()} aria-busy={busy() ? 'true' : undefined} onClick={applyPair}>
          <Show when={busy()}>
            <span class="mp-spinner" aria-hidden="true" />
          </Show>
          {busy() ? 'Applying…' : pairUnchanged() ? 'Current pair' : fusionOn() ? 'Update pair' : 'Use this pair'}
        </button>
      </Show>
      <Show when={mode() === 'single'}>
        <Show
          when={action()}
          fallback={<p class="mp-hint">{fusionOn() ? 'Fusion is on. Pick a model to switch to it.' : (desktopShell() ? 'Click a model to switch.' : 'Tap a model to switch.')}</p>}
        >
          {(act) => (
            <div class="flex gap-3">
              <button type="button" class="pair-secondary flex-1" disabled={busy()} onClick={() => (setPick(undefined), setError(null))}>
                Cancel
              </button>
              <button type="button" class="pair-primary flex-[1.6]" disabled={busy()} aria-busy={busy() ? 'true' : undefined} onClick={applySingle}>
                <Show when={busy()}>
                  <span class="mp-spinner" aria-hidden="true" />
                </Show>
                {busy() ? 'Switching…' : act().label}
              </button>
            </div>
          )}
        </Show>
      </Show>
    </div>
  );

  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Model" testId="chat-model-picker" class="model-picker-sheet" footer={footer}>
      <p class="mp-current" data-testid="model-picker-current">
        <span class="mp-current-label">Now</span>
        <span class="truncate">{currentLabel()}</span>
      </p>
      <Show when={s().fusion}>
        <div class="think-levels mp-modes" role="radiogroup" aria-label="Model mode">
          <button type="button" role="radio" aria-checked={mode() === 'single'} class="think-level" onClick={() => (setMode('single'), setError(null))}>
            Single
          </button>
          <button type="button" role="radio" aria-checked={mode() === 'fusion'} class="think-level" onClick={() => (setMode('fusion'), setError(null))}>
            Fusion
          </button>
        </div>
      </Show>

      <Show when={mode() === 'fusion' && s().fusion}>
        <div class="fusion-pair" ref={(el) => queueMicrotask(() => revealPicked(el))}>
          <div class="fusion-side">
            <p class="sheet-label">Lead</p>
            <div class="model-list cc-fusion-list">
              <For each={s().fusion!.leads}>
                {(m) => (
                  <button type="button" class="model-row" aria-current={leadKey() === m.key ? 'true' : undefined} onClick={() => (props.onHaptic(), setError(null), setPendingLead(m.key))}>
                    <span class="min-w-0 flex-1 text-left fusion-model-name">{shortModelName(m.name)}</span>
                    <Show when={leadKey() === m.key}>
                      <Check size={16} class="board-accent shrink-0" />
                    </Show>
                  </button>
                )}
              </For>
            </div>
          </div>
          <div class="fusion-side">
            <p class="sheet-label">Sidekick</p>
            <div class="model-list cc-fusion-list">
              <For each={s().fusion!.sidekicks}>
                {(m) => (
                  <button type="button" class="model-row" aria-current={sidekickKey() === m.key ? 'true' : undefined} onClick={() => (props.onHaptic(), setError(null), setPendingSidekick(m.key))}>
                    <span class="min-w-0 flex-1 text-left fusion-model-name">{shortModelName(m.name)}</span>
                    <Show when={sidekickKey() === m.key}>
                      <Check size={16} class="board-accent shrink-0" />
                    </Show>
                  </button>
                )}
              </For>
            </div>
          </div>
        </div>
        <p class="sheet-label">Effort</p>
        <div class="think-levels" role="radiogroup" aria-label="Lead effort">
          <For each={['low', 'medium', 'high']}>
            {(level) => (
              <button type="button" role="radio" aria-checked={leadEffort() === level} class="think-level" onClick={() => setLeadEffort(level)}>
                lead {level}
              </button>
            )}
          </For>
        </div>
        <div class="think-levels" role="radiogroup" aria-label="Sidekick effort">
          <For each={['low', 'medium', 'high']}>
            {(level) => (
              <button type="button" role="radio" aria-checked={sidekickEffort() === level} class="think-level" onClick={() => setSidekickEffort(level)}>
                side {level}
              </button>
            )}
          </For>
        </div>
      </Show>

      <Show when={mode() === 'single'}>
        <Show when={!fusionOn() && s().run.model?.reasoning && (s().run.thinkingLevels?.length ?? 0) > 1}>
          <p class="sheet-label">
            <Brain size={14} /> Thinking
          </p>
          <div class="think-levels" role="radiogroup" aria-label="Thinking level">
            <For each={s().run.thinkingLevels ?? []}>
              {(level) => (
                <button type="button" role="radio" aria-checked={s().run.thinking === level} class="think-level" onClick={() => setThinking(level)}>
                  {level}
                </button>
              )}
            </For>
          </div>
        </Show>
        <p class="sheet-label">
          <Cpu size={14} /> Model
        </p>
        <Show when={s().models.length > 6}>
          <input class="model-search" placeholder={`Search all ${s().models.length} models`} value={q()} onInput={(e) => setQ(e.currentTarget.value)} />
        </Show>
        <div class="model-list mp-model-list" data-testid="model-picker-list">
          <For each={models().slice(0, 120)} fallback={<p class="board-menu-note">No model matches.</p>}>
            {(m) => {
              const key = `${m.provider}/${m.id}`;
              const cur = () => current().kind === 'single' && (current() as { key?: string }).key === key;
              const picked = () => pick() === key;
              return (
                <button
                  type="button"
                  class="model-row"
                  aria-current={cur() ? 'true' : undefined}
                  aria-pressed={picked() ? 'true' : 'false'}
                  data-picked={picked() ? '' : undefined}
                  onClick={() => {
                    props.onHaptic();
                    setError(null);
                    setPick(nextSinglePick(current(), pick(), key));
                  }}
                >
                  <span class="min-w-0 flex-1 text-left">
                    <span class="block truncate">{m.name ?? m.id}</span>
                    <span class="block truncate model-row-sub">
                      {m.provider}/{m.id}
                    </span>
                  </span>
                  <Show when={cur()}>
                    <span class="mp-tag">current</span>
                  </Show>
                  <Show when={picked()}>
                    <Check size={18} class="board-accent shrink-0" />
                  </Show>
                </button>
              );
            }}
          </For>
        </div>
      </Show>
    </BoardSheet>
  );
}

/** Each Fusion column is a short scrolling list; the lead/sidekick in use often sits below its fold
 * (seen on the Redmi: neither column showed a pick). Scroll each list so its picked row shows,
 * without moving the sheet itself. */
export function revealPicked(root: HTMLElement): void {
  for (const list of root.querySelectorAll<HTMLElement>('.cc-fusion-list')) {
    const row = list.querySelector<HTMLElement>('[aria-current="true"]');
    if (!row) continue;
    const lr = list.getBoundingClientRect();
    const rr = row.getBoundingClientRect();
    if (rr.top < lr.top || rr.bottom > lr.bottom) {
      list.scrollTop += rr.top - lr.top - (lr.height - rr.height) / 2;
    }
  }
}

/** The Session sheet (UNI-201): Context / Running / Info tabs — opened
 * from the header title tap, the header ⋯ menu rows and the tray rows
 * (everything except the model picker, which has its own sheet). */
export function ControlCentreSheet(props: { open: boolean; onClose: () => void; chat: Chat; onHaptic: () => void; initialSection?: Section }) {
  const [section, setSection] = createSignal<Section>(props.initialSection ?? 'context');
  createEffect(() => {
    if (props.open) setSection(props.initialSection ?? 'context');
  });
  const [toast, setToast] = createSignal<string | null>(null);
  const showToast = (text: string) => {
    setToast(text);
    setTimeout(() => setToast(null), 3500);
  };

  return (
    <BoardSheet open={props.open} onClose={props.onClose} label="Session" testId="chat-control-centre">
      <div class="cc-tabs" role="tablist" aria-label="Session sections">
        <For each={SECTIONS}>
          {(sec) => (
            <button type="button" role="tab" aria-selected={section() === sec.id} class="cc-tab" onClick={() => (props.onHaptic(), setSection(sec.id))}>
              <sec.icon size={16} />
              {sec.label}
            </button>
          )}
        </For>
      </div>
      <Show when={section() === 'context'}>
        <ContextSection chat={props.chat} />
      </Show>
      <Show when={section() === 'running'}>
        <RunningSection chat={props.chat} onToast={showToast} />
      </Show>
      <Show when={section() === 'info'}>
        <InfoSection chat={props.chat} />
      </Show>
      <Show when={toast()}>
        <div class="chat-toast cc-toast" role="status">
          {toast()}
        </div>
      </Show>
    </BoardSheet>
  );
}

export { formatDuration };
export type { Section };
