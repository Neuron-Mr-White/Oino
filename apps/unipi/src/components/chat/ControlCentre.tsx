/**
 * Session sheet (UNI-160/UNI-201; WP-A14 split): Context / Running / Info
 * only. WP-A14 took the model picker out to `components/model/` (the v1
 * `ModelPickerSheet` is gone) and deleted the header strip and the tray
 * (Z-75: no `FooterStrip` / `FooterTray` — the cost sits on the turn's end,
 * the model / thinking / context live in the composer and the Session modal).
 * WP-A17 superseded this sheet with `SessionModal`; the file stays until the
 * v1 chat screens are pruned.
 */
import { createEffect, createSignal, For, onCleanup, Show } from 'solid-js';
import { ChevronRight, Cpu, Gauge, ListChecks, Info as InfoIcon, X } from 'lucide-solid';
import { BoardSheet } from '../board/BoardSheet';
import { formatCount, formatUsd } from '../session/modalData';
import type { Chat, WorkItem } from '../../lib/chat/store';

/** Session sheet tabs. */
type Section = 'context' | 'running' | 'info';

const SECTIONS: Array<{ id: Section; label: string; icon: typeof Cpu }> = [
  { id: 'context', label: 'Context', icon: Gauge },
  { id: 'running', label: 'Running', icon: ListChecks },
  { id: 'info', label: 'Info', icon: InfoIcon },
];

export const DOT_LABEL: Record<WorkItem['dot'], string> = { running: 'Running', stopped: 'Stopped', failed: 'Failed', done: 'Done' };

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

/** What pi really runs right now (UNI-212): Fusion only when the preset's
 * `active` says so AND pi's model is still its lead — a /model switch in the
 * TUI (or an older bridge that didn't push the fresh preset) must never
 * leave the picker marking Fusion as current. */
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

export type { Section };
