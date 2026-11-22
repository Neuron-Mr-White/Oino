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
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from 'solid-js';
import { ChevronDown, ChevronUp, GripVertical, Image as ImageIcon, MoreVertical, Pencil, Play, SendHorizonal, X, Zap } from 'lucide-solid';
import type { SessionHandle } from '../../lib/session/store';
import type { OutboxImage, QueueState } from '../../lib/session/types';
import { composerDraft, focusComposer, setComposerText } from '../../lib/composer/drafts';
import { escalationView, kindView } from '../../lib/composer/queueEscalate';
import { queueImageKey } from '../../lib/session/outbox';
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

/** `[image 2]` tokens in a queued text (the host's stand-in for a pasted image). */
const IMAGE_TOKEN = /\[image (\d+)\]/g;

/** A queued text split into plain text and its image tokens, each paired with the image this client holds (if any). */
export type QueueTextPart = { text: string } | { image: OutboxImage | undefined; n: number };
export function queueTextParts(text: string, images?: readonly OutboxImage[]): QueueTextPart[] {
  const parts: QueueTextPart[] = [];
  let at = 0;
  for (const m of text.matchAll(IMAGE_TOKEN)) {
    if (m.index! > at) parts.push({ text: text.slice(at, m.index) });
    const n = Number(m[1]);
    parts.push({ image: images?.[n - 1], n });
    at = m.index! + m[0].length;
  }
  if (at < text.length) parts.push({ text: text.slice(at) });
  return parts;
}

/** How long the "Confirm" on a stop-and-send button waits for the second press. */
export const CONFIRM_MS = 3000;

export function QueuePanel(props: { session: SessionHandle; phone?: boolean }) {
  const q = () => props.session.state.queue;
  const items = createMemo(() => queueItems(q()));
  const running = () => !!props.session.state.run.running;
  const restoreFailed = () => props.session.state.outbox.filter((item) => item.queueRestoreFailed && item.status === 'lost');
  /** After a stop the leftover queue is held until Resume (or any new run). */
  const held = () => !running() && items().length > 0;

  const edit = (op: Record<string, unknown>) => {
    const queue = op.queue as 'steering' | 'followUp';
    const index = (op.index ?? op.from) as number;
    const messageId = (queue === 'steering' ? q().steeringIds : q().followUpIds)?.[index];
    return props.session.cmd({ type: '_oino/queue_edit', ...(messageId ? { op: 'byId', messageId, edit: op } : op) });
  };
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
    const images = imagesOf(item);
    void edit({ op: 'remove', queue: item.queue, index: item.index })
      .then(() => props.session.abort())
      .then(() => void props.session.send(item.text, { mode: 'prompt', ...(images?.length ? { images } : {}) }))
      .catch(() => undefined);
  };

  /** The pasted images this client still holds for a queued row (paired by the host's message id). */
  const imagesOf = (item: QueueItem): OutboxImage[] | undefined => {
    const id = (item.queue === 'steering' ? q().steeringIds : q().followUpIds)?.[item.index];
    const held = props.session.state.queueImages;
    return held?.[queueImageKey(id, item.text)] ?? held?.[queueImageKey(undefined, item.text)];
  };

  /**
   * D-8: stopping the agent's step is not undoable, so its button asks twice: the first press turns it into
   * "Confirm" for `CONFIRM_MS`, the second one (inside that window) sends. Any other row's press disarms it.
   */
  const [armed, setArmed] = createSignal<string>();
  let armTimer: ReturnType<typeof setTimeout> | undefined;
  const disarm = () => {
    clearTimeout(armTimer);
    armTimer = undefined;
    setArmed(undefined);
  };
  onCleanup(() => clearTimeout(armTimer));
  const itemKey = (item: QueueItem) => `${item.queue}:${item.index}:${item.text}`;
  const press = (item: QueueItem) => {
    if (escalationView(item.queue, running()).step !== 'interrupt') {
      disarm();
      escalate(item);
      return;
    }
    if (armed() === itemKey(item)) {
      disarm();
      escalate(item);
      return;
    }
    disarm();
    setArmed(itemKey(item));
    armTimer = setTimeout(disarm, CONFIRM_MS);
  };

  // ── phone: the row's actions live in one ⋮ menu so the text keeps the width ──
  const [menuFor, setMenuFor] = createSignal<string>();
  const closeMenu = () => {
    setMenuFor(undefined);
    disarm();
  };
  const onDocPointerDown = (e: PointerEvent) => {
    if (!menuFor()) return;
    if (e.composedPath?.().some((t) => t instanceof Element && t.closest?.('[data-queue-menu-root]'))) return;
    closeMenu();
  };
  document.addEventListener('pointerdown', onDocPointerDown, true);
  // N-2: Esc with the ⋮ menu open closes the menu and is consumed (a popup took it, Z-55) — it must not also stop
  // the run. Capture on the document, before the conversation root's stop rule, whatever has focus.
  const onDocKeyDown = (e: KeyboardEvent) => {
    if (e.key !== 'Escape' || !menuFor()) return;
    e.preventDefault();
    e.stopPropagation();
    closeMenu();
  };
  document.addEventListener('keydown', onDocKeyDown, true);
  onCleanup(() => {
    document.removeEventListener('pointerdown', onDocPointerDown, true);
    document.removeEventListener('keydown', onDocKeyDown, true);
  });

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
    <>
      <Show when={restoreFailed().length || props.session.state.queueRestoreError}>
        <div class="dock-queue" role="alert" data-testid="queue-restore-failed">
          <div class="dock-queue-bar">Queued messages could not be restored</div>
          <p>{props.session.state.queueRestoreError} New messages can still be sent.</p>
          <ul class="dock-queue-list">
            <For each={restoreFailed()}>{(item) => (
              <li class="dock-queue-row">
                <span class="dock-queue-text">{item.text}</span>
                <button type="button" class="dock-queue-resume" onClick={() => props.session.resend(item.ref)}>Send again</button>
              </li>
            )}</For>
          </ul>
        </div>
      </Show>
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
                <span class="dock-queue-kind" data-testid="queue-kind" data-held={held() ? '' : undefined} title={held() ? 'Waiting — press Resume to send it' : kindView(item.queue).tooltip}>
                  {held() ? 'Held' : kindView(item.queue).label}
                </span>
                <span class="dock-queue-text" data-testid="queue-text" title={item.text}>
                  <For each={queueTextParts(item.text, imagesOf(item))}>{(part) =>
                    'text' in part ? part.text : part.image ? (
                      <img class="dock-queue-thumb" data-testid="queue-thumb" src={`data:${part.image.mimeType};base64,${part.image.data}`} alt={`image ${part.n}`} />
                    ) : (
                      <span class="dock-queue-imgchip" data-testid="queue-image-chip"><ImageIcon size={11} /> image</span>
                    )
                  }</For>
                </span>
                <Show when={props.phone}>
                  <div class="dock-queue-more" data-queue-menu-root>
                    <button
                      type="button"
                      class="dock-queue-icon"
                      aria-label="Message actions"
                      aria-haspopup="menu"
                      aria-expanded={menuFor() === itemKey(item)}
                      onClick={() => (menuFor() === itemKey(item) ? closeMenu() : (disarm(), setMenuFor(itemKey(item))))}
                      data-testid="queue-more"
                    >
                      <MoreVertical size={16} />
                    </button>
                    <Show when={menuFor() === itemKey(item)}>
                      <div class="dock-queue-menu" role="menu" aria-label="Message actions" data-testid="queue-menu">
                        <button type="button" role="menuitem" class="dock-queue-menu-item" disabled={item.index === 0} onClick={() => (move1(item, -1), closeMenu())}>
                          <ChevronUp size={15} /> Move up
                        </button>
                        <button type="button" role="menuitem" class="dock-queue-menu-item" disabled={item.index >= q()[item.queue].length - 1} onClick={() => (move1(item, 1), closeMenu())}>
                          <ChevronDown size={15} /> Move down
                        </button>
                        <button type="button" role="menuitem" class="dock-queue-menu-item" data-testid="queue-edit" onClick={() => (closeMenu(), recall(item))}>
                          <Pencil size={14} /> Edit
                        </button>
                        <button
                          type="button"
                          role="menuitem"
                          class="dock-queue-menu-item"
                          data-testid="queue-send-now"
                          data-step={escalationView(item.queue, running()).step}
                          data-armed={armed() === itemKey(item) ? '' : undefined}
                          title={escalationView(item.queue, running()).tooltip}
                          onClick={() => {
                            const stays = armed() !== itemKey(item) && escalationView(item.queue, running()).step === 'interrupt';
                            press(item);
                            if (!stays) closeMenu();
                          }}
                        >
                          {escalationView(item.queue, running()).step === 'interrupt' ? <Zap size={14} /> : <SendHorizonal size={14} />}
                          {armed() === itemKey(item) ? 'Confirm — stop and send' : escalationView(item.queue, running()).label}
                        </button>
                        <button type="button" role="menuitem" class="dock-queue-menu-item" data-testid="queue-remove" onClick={() => (closeMenu(), void edit({ op: 'remove', queue: item.queue, index: item.index }).catch(() => undefined))}>
                          <X size={14} /> Remove from queue
                        </button>
                      </div>
                    </Show>
                  </div>
                </Show>
                <Show when={!props.phone}>
                <button
                  type="button"
                  class="dock-queue-icon"
                  aria-label="Edit"
                  title={hasComposerDraft(draftKey()) ? 'Edit — the composer has a draft, send or clear it first' : 'Edit'}
                  onClick={() => recall(item)}
                  data-testid="queue-edit"
                >
                  <Pencil size={14} />
                </button>
                  <button
                    type="button"
                    class="dock-queue-icon"
                    title={armed() === itemKey(item) ? 'Press again within 3 s to stop the current step and send now' : escalationView(item.queue, running()).tooltip}
                    data-step={escalationView(item.queue, running()).step}
                    aria-label={armed() === itemKey(item) ? 'Confirm: stop the current step and send now' : escalationView(item.queue, running()).label}
                    onClick={() => press(item)}
                    data-testid="queue-send-now"
                    data-armed={armed() === itemKey(item) ? '' : undefined}
                  >
                    {armed() === itemKey(item) ? 'Confirm stop & send' : escalationView(item.queue, running()).step === 'interrupt' ? <Zap size={14} /> : <SendHorizonal size={14} />}
                  </button>
                <button
                  type="button"
                  class="dock-queue-icon"
                  aria-label="Remove from queue"
                  title="Remove from queue"
                  onClick={() => void edit({ op: 'remove', queue: item.queue, index: item.index }).catch(() => undefined)}
                  data-testid="queue-remove"
                >
                  <X size={14} />
                </button>
                </Show>
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
    </>
  );
}
