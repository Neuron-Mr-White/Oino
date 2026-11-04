/**
 * The queue panel of the bottom dock (WP-A11, Z-53 — ruling: ZCode's
 * recall-to-composer + draft guard and held-queue Resume, unipi's phone
 * arrows): everything the host holds queued for this session — steering
 * texts (delivered at the next tool boundary) and follow-ups (delivered
 * when the run ends), as `queue_update` carries them.
 *
 *   · desktop rows drag to reorder; phone rows use ↑ / ↓
 *   · Edit recalls one item into the composer (refused while a draft
 *     exists — nothing may clobber typed text), then removes it from
 *     the queue
 *   · Remove and Send now edit one item (`_oino/queue_edit`);
 *     Clear empties both queues (`clear_queue`)
 *   · after a stop leaves the queue behind, it is **held**: Resume
 *     delivers the first item now and the run drains the rest
 *     (auto-drain).
 */
import { createEffect, createMemo, createSignal, For, Show } from 'solid-js';
import { ChevronDown, ChevronUp, GripVertical, Pencil, Play, SendHorizonal, X, Zap } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import type { QueueState } from '../../lib/session/types';
import { composerDraft, focusComposer, setComposerText } from '../../lib/composer/drafts';
import { escalationView } from '../../lib/composer/queueEscalate';
import '../dock/dock.css';

/** One drawn row: which queue, its index there, its text. */
export type QueueItem = { queue: 'steering' | 'followUp'; index: number; text: string };

export const queueItems = (q: QueueState): QueueItem[] => [
  ...q.steering.map((text, index) => ({ queue: 'steering' as const, index, text })),
  ...q.followUp.map((text, index) => ({ queue: 'followUp' as const, index, text })),
];

/** `move` on the item's own queue (same queue only; other-queue drops are refused). */
export function moveEdit(item: QueueItem, target: QueueItem): { op: 'move'; queue: QueueItem['queue']; from: number; to: number } | undefined {
  if (item.queue !== target.queue || item.index === target.index) return undefined;
  return { op: 'move', queue: item.queue, from: item.index, to: target.index };
}

/** Applies a move the way the host does: remove at `from`, insert at `to`. */
export function applyMove<T>(list: readonly T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) return [...list];
  const next = [...list];
  const [moved] = next.splice(from, 1);
  next.splice(to, 0, moved!);
  return next;
}

/** The composer holds unsent input: Edit recall refuses (Z-53 draft guard). */
export function hasComposerDraft(key: string): boolean {
  const d = composerDraft(key);
  return !!(d.text.trim() || d.quotes.length || d.images.length || d.chips.length);
}

export function QueuePanel(props: { session: SessionHandle; phone?: boolean }) {
  const q = () => props.session.state.queue;
  const items = createMemo(() => queueItems(q()));
  const running = () => !!props.session.state.run.running;
  /** After a stop the leftover queue is held until Resume (or any new run). */
  const held = () => !running() && items().length > 0;

  const edit = (op: Record<string, unknown>) => props.session.cmd({ type: '_oino/queue_edit', ...op });
  const clear = () => void props.session.cmd({ type: 'clear_queue' }).catch(() => undefined);

  /** Resume: deliver the first item now — the run it starts drains the rest. */
  const resume = () => {
    const first = items()[0];
    if (!first) return;
    void edit({ op: 'sendNow', queue: first.queue, index: first.index }).catch(() => undefined);
  };

  /**
   * One press of an item's send button (owner-fix-3): QUEUE → STEER
   * (`sendNow` while running makes it the first steering message), STEER →
   * interrupt: drop it from the queue, abort the run, send it as a prompt.
   */
  const escalate = (item: QueueItem) => {
    const step = escalationView(item.queue, running()).step;
    if (step !== 'interrupt') {
      void edit({ op: 'sendNow', queue: item.queue, index: item.index }).catch(() => undefined);
      return;
    }
    void edit({ op: 'remove', queue: item.queue, index: item.index })
      .then(() => props.session.abort())
      .then(() => void props.session.send(item.text, { mode: 'prompt' }))
      .catch(() => undefined);
  };

  // ── Edit = recall into the composer (Z-53 draft guard) ───────────────
  const [refused, setRefused] = createSignal<string>();
  const draftKey = () => props.session.key;
  // Once the composer is free again the refusal notice is stale.
  createEffect(() => {
    if (!hasComposerDraft(draftKey())) setRefused(undefined);
  });
  const recall = (item: QueueItem) => {
    if (hasComposerDraft(draftKey())) {
      setRefused(item.text);
      return;
    }
    setRefused(undefined);
    void edit({ op: 'remove', queue: item.queue, index: item.index })
      .then(() => {
        setComposerText(draftKey(), item.text);
        focusComposer(draftKey());
      })
      .catch(() => undefined);
  };

  // ── desktop drag reorder ─────────────────────────────────────────────
  const [drag, setDrag] = createSignal<QueueItem>();
  const [over, setOver] = createSignal<QueueItem>();
  const drop = (target: QueueItem) => {
    const from = drag();
    setDrag(undefined);
    setOver(undefined);
    if (!from) return;
    const e = moveEdit(from, target);
    if (e) void edit(e).catch(() => undefined);
  };

  const move1 = (item: QueueItem, by: -1 | 1) => {
    const list = q()[item.queue];
    const to = item.index + by;
    if (to < 0 || to >= list.length) return;
    void edit({ op: 'move', queue: item.queue, from: item.index, to }).catch(() => undefined);
  };

  return (
    <Show when={items().length}>
      <div class="dock-queue" data-testid="queue-panel" data-held={held() ? '' : undefined} data-phone={props.phone ? '' : undefined}>
        <div class="dock-queue-bar">
          <span class="dock-queue-title" data-testid="queue-count">
            {held() ? 'Held' : 'Queue'} · {items().length}
          </span>
          <Show when={held()}>
            <button type="button" class="dock-queue-resume" onClick={resume} data-testid="queue-resume">
              <Play size={13} /> Resume
            </button>
          </Show>
          <span class="flex-1" />
          <Show when={items().length > 1}>
            <button type="button" class="dock-queue-clear" onClick={clear} data-testid="queue-clear">
              Clear
            </button>
          </Show>
        </div>
        <ul class="dock-queue-list" role="list">
          <For each={items()}>
            {(item) => (
              <li
                class="dock-queue-row"
                data-kind={item.queue}
                data-over={over() === item ? '' : undefined}
                draggable={props.phone ? undefined : true}
                onDragStart={(e) => {
                  setDrag(item);
                  e.dataTransfer?.setData('text/plain', item.text);
                  if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
                }}
                onDragOver={(e) => {
                  if (!drag()) return;
                  e.preventDefault();
                  setOver(item);
                }}
                onDragEnd={() => {
                  setDrag(undefined);
                  setOver(undefined);
                }}
                onDrop={(e) => {
                  e.preventDefault();
                  drop(item);
                }}
              >
                <Show when={!props.phone}>
                  <span class="dock-queue-grip" aria-hidden title="Drag to reorder">
                    <GripVertical size={14} />
                  </span>
                </Show>
                <span class="dock-queue-kind" title={item.queue === 'steering' ? 'Delivered at the next tool boundary' : 'Sent after this run ends'}>
                  {item.queue === 'steering' ? 'Steer' : 'Queue'}
                </span>
                <span class="dock-queue-text">{item.text}</span>
                <Show when={props.phone}>
                  <button type="button" class="dock-queue-icon" aria-label="Move up" disabled={item.index === 0} onClick={() => move1(item, -1)}>
                    <ChevronUp size={15} />
                  </button>
                  <button
                    type="button"
                    class="dock-queue-icon"
                    aria-label="Move down"
                    disabled={item.index >= q()[item.queue].length - 1}
                    onClick={() => move1(item, 1)}
                  >
                    <ChevronDown size={15} />
                  </button>
                </Show>
                <button
                  type="button"
                  class="dock-queue-icon"
                  aria-label={`Edit ${item.text}`}
                  title={hasComposerDraft(draftKey()) ? 'The composer has a draft — send or clear it first' : 'Edit in the composer'}
                  onClick={() => recall(item)}
                  data-testid="queue-edit"
                >
                  <Pencil size={14} />
                </button>
                  <button
                    type="button"
                    class="dock-queue-icon"
                    title={escalationView(item.queue, running()).tooltip}
                    data-step={escalationView(item.queue, running()).step}
                    aria-label={`${escalationView(item.queue, running()).label}: ${item.text}`}
                    onClick={() => escalate(item)}
                    data-testid="queue-send-now"
                  >
                    {escalationView(item.queue, running()).step === 'interrupt' ? <Zap size={14} /> : <SendHorizonal size={14} />}
                  </button>
                <button
                  type="button"
                  class="dock-queue-icon"
                  aria-label={`Remove ${item.text}`}
                  onClick={() => void edit({ op: 'remove', queue: item.queue, index: item.index }).catch(() => undefined)}
                  data-testid="queue-remove"
                >
                  <X size={14} />
                </button>
              </li>
            )}
          </For>
        </ul>
        <Show when={refused()}>
          <p class="dock-queue-refused" role="status">
            The composer has a draft — send or clear it before editing “{refused()}”.
          </p>
        </Show>
      </div>
    </Show>
  );
}
