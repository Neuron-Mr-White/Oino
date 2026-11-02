/**
 * Turn tail (app spec WP-A7, Z-17/Z-26/Z-27/Z-49/Z-93): one line at the
 * bottom-left of a settled turn —
 *
 *   [Copy] [Quote] [Fork] [Retry] [More]   Done in 1m 3s · $0.003 · 09:41
 *
 * The action buttons are hover-revealed on the desktop and always visible on
 * the phone (§3.9); the done/cost/time line is always visible. Copy copies the
 * whole turn (query + steers + answers) and flips its glyph to a check for
 * 1.2 s (Z-93). Retry appears on the last turn only and re-runs it from its
 * user message (`_oino/navigate_tree` + `prompt`, Z-49). Fork uses pi's fork
 * (before the turn's user message) and routes to the fork (Z-17).
 *
 * Per-message actions live in the right-click / long-press menu (`TurnMenus`);
 * Edit on a user message tree-jumps into the composer, with A27's "Also undo
 * file changes" once that capability exists (Z-28).
 */
import { createEffect, createSignal, on, onCleanup, Show } from 'solid-js';
import { Check, Copy, Ellipsis, FileCode, Pencil, Quote, RotateCcw, Share2, SquareStack, TextSelect } from 'lucide-solid';
import { layout } from '../../lib/layout';
import { ActionSheet, haptic, type ActionItem } from '../phone';
import { formatDuration } from '../../lib/chat/model';
import { copyText } from '../../lib/board/share';
import type { Turn } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import {
  actionError,
  closeEditRequest,
  closeMessageMenu,
  composerDraft,
  currentEditRequest,
  currentMessageMenu,
  editMessage,
  entryImages,
  focusComposer,
  forkFrom,
  messageText,
  openEditRequest,
  openMessageMenu,
  quoteIntoComposer,
  restoreComposer,
  retryTurn,
  shareTurn,
  showActionNotice,
  treeJump,
  turnQuoteText,
  turnText,
  turnTime,
} from '../../lib/timeline/actions';
import { AnchoredMenu, MenuLabel, MenuRow } from '../board/AnchoredMenu';
import type { TimelineCtx } from './context';
import { TimeLabel } from './TimeLabel';

const COPIED_MS = 1_200;

async function copyWithFallback(text: string): Promise<boolean> {
  if (await copyText(text)) return true;
  if (typeof document === 'undefined') return false;
  try {
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    ta.style.pointerEvents = 'none';
    document.body.appendChild(ta);
    ta.select();
    const ok = typeof document.execCommand === 'function' && document.execCommand('copy');
    ta.remove();
    return ok;
  } catch {
    return false;
  }
}

/** Copy with the 1.2 s glyph→check feedback (Z-93). */
function copyFeedback(text: () => string, what: string) {
  const [copied, setCopied] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  onCleanup(() => timer && clearTimeout(timer));
  const copy = async () => {
    if (!(await copyWithFallback(text()))) {
      showActionNotice(`Couldn't copy ${what}.`);
      return;
    }
    setCopied(true);
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => setCopied(false), COPIED_MS);
  };
  return { copied, copy };
}

export function TurnActions(props: { turn: Turn; ctx: TimelineCtx; last?: boolean }) {
  const turn = () => props.turn;
  const copy = copyFeedback(() => turnText(turn()), 'the turn');
  const [more, setMore] = createSignal<DOMRect | undefined>();
  const running = () => props.ctx.session.state.run.running;
  const done = () => {
    const t = turn();
    if (t.stopped) return 'Stopped';
    if (t.error) return `Failed${t.durationMs !== undefined ? ` after ${formatDuration(t.durationMs)}` : ''}`;
    if (t.durationMs !== undefined) return `Done in ${formatDuration(t.durationMs)}`;
    return 'Done';
  };
  const cost = () => {
    const value = turn().costUsd;
    return value !== undefined && value >= 0.0005 ? `$${value.toFixed(value < 0.1 ? 3 : 2)}` : undefined;
  };
  const fork = async () => {
    const user = turn().user;
    try {
      await forkFrom(props.ctx.hostId, props.ctx.session.sessionId(), user?.id, props.ctx.openSession);
      showActionNotice('Forked into a new chat.');
    } catch (error) {
      showActionNotice(`Couldn't fork: ${actionError(error)}`);
    }
  };
  const retry = async () => {
    try {
      await retryTurn(props.ctx.session, turn());
    } catch (error) {
      showActionNotice(`Couldn't retry: ${actionError(error)}`);
    }
  };
  const quote = () => quoteIntoComposer(props.ctx.session.key, turnQuoteText(turn()));
  const share = async () => {
    try {
      await shareTurn(turn());
    } catch (error) {
      showActionNotice(`Couldn't share: ${actionError(error)}`);
    }
  };
  const phone = () => !layout().desktop();
  return (
    <Show
      when={!phone()}
      fallback={
        // Phone (§3.4, PH-26): no per-turn footer — only the last settled turn shows
        // time · Copy · Retry · ⋯; every other action is a long-press on the message.
        <Show when={props.last && turn().kind === 'turn' && !turn().live}>
          <div class="tl-tail-line ph-turn-tail" data-testid="tl-turn-tail">
            <Show when={turnTime(turn()) !== undefined}>
              <TimeLabel at={turnTime(turn())} />
            </Show>
            <div class="tl-actions" role="toolbar" aria-label="Turn actions">
              <button type="button" class="tl-act" data-testid="tl-copy" data-copied={copy.copied() ? '' : undefined} aria-label={copy.copied() ? 'Copied' : 'Copy turn'} onClick={() => void copy.copy()}>
                <Show when={copy.copied()} fallback={<Copy size={20} />}>
                  <Check size={20} class="tl-act-check" />
                </Show>
              </button>
              <button type="button" class="tl-act" data-testid="tl-retry" aria-label="Retry this turn" disabled={running()} onClick={() => void retry()}>
                <RotateCcw size={20} />
              </button>
              <button
                type="button"
                class="tl-act"
                data-testid="tl-more"
                aria-label="More turn actions"
                onClick={(event) => {
                  const answer = turnText(turn());
                  openMessageMenu(event.currentTarget.getBoundingClientRect(), { text: answer, role: 'assistant', userEntryId: turn().user?.id, turnId: turn().turnId });
                }}
              >
                <Ellipsis size={20} />
              </button>
            </div>
          </div>
        </Show>
      }
    >
    <Show when={turn().kind === 'turn' && !turn().live}>
      <div class="tl-tail-line" data-testid="tl-turn-tail">
        <div class="tl-meta">
          <span class="tl-done" data-stopped={turn().stopped ? '' : undefined} data-error={turn().error ? '' : undefined} data-testid="tl-turn-done">
            {done()}
          </span>
          <Show when={cost()}>
            <span class="tl-done-sep" aria-hidden="true">·</span>
            <span class="tl-done" data-testid="tl-turn-cost">
              {cost()}
            </span>
          </Show>
          <Show when={turnTime(turn()) !== undefined}>
            <span class="tl-done-sep" aria-hidden="true">·</span>
            <TimeLabel at={turnTime(turn())} />
          </Show>
        </div>
        <div class="tl-actions" role="toolbar" aria-label="Turn actions">
          <button
            type="button"
            class="tl-act"
            data-testid="tl-copy"
            data-copied={copy.copied() ? '' : undefined}
            aria-label={copy.copied() ? 'Copied' : 'Copy turn'}
            data-tip={copy.copied() ? 'Copied' : 'Copy turn'}
            onClick={() => void copy.copy()}
          >
            <Show when={copy.copied()} fallback={<Copy size={14} />}>
              <Check size={14} class="tl-act-check" />
            </Show>
          </button>
          <button type="button" class="tl-act" data-testid="tl-quote" aria-label="Quote this turn" data-tip="Quote" onClick={quote}>
            <Quote size={14} />
          </button>
          <button type="button" class="tl-act" data-testid="tl-fork" aria-label="Fork from this turn" data-tip="Fork from here" onClick={() => void fork()}>
            <SquareStack size={14} />
          </button>
          <Show when={props.last}>
            <button
              type="button"
              class="tl-act"
              data-testid="tl-retry"
              aria-label="Retry this turn"
              data-tip="Retry"
              disabled={running()}
              onClick={() => void retry()}
            >
              <RotateCcw size={14} />
            </button>
          </Show>
          <button
            type="button"
            class="tl-act"
            data-testid="tl-more"
            aria-label="More turn actions"
            data-tip="More"
            onClick={(event) => setMore(event.currentTarget.getBoundingClientRect())}
          >
            <Ellipsis size={14} />
          </button>
        </div>
        <Show when={more()} keyed>
          {(anchor) => (
            <AnchoredMenu anchor={anchor} onDismiss={() => setMore(undefined)} label="More turn actions" width={224} alignEnd>
              <MenuRow
                icon={<Pencil size={18} />}
                label="Edit message"
                disabled={!turn().user}
                onSelect={() => {
                  const user = turn().user;
                  setMore(undefined);
                  if (user) void requestEdit(props.ctx, turn(), user, anchor);
                }}
              />
              <MenuRow icon={<Share2 size={18} />} label="Share turn" onSelect={() => (setMore(undefined), void share())} />
            </AnchoredMenu>
          )}
        </Show>
      </div>
    </Show>
    </Show>
  );
}

/** Markdown → the plain text a phone "Copy" gives (Copy as Markdown keeps the source). */
export function plainOfMarkdown(md: string): string {
  return md
    .replace(/```[^\n]*\n([\s\S]*?)```/g, '$1')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/^#{1,6}\s+/gm, '')
    .replace(/^\s*>\s?/gm, '')
    .replace(/(\*\*|__)(.+?)\1/g, '$2')
    .replace(/(^|[^*\w])[*_]([^*_\n]+)[*_](?=[^*\w]|$)/g, '$1$2')
    .replace(/^\s*[-*+]\s+/gm, '• ')
    .trim();
}

/**
 * Phone (§3.4, PH-26): the long-press sheet of a message. Assistant: Copy · Copy as
 * Markdown · Quote in reply · Fork from here · Retry · Select text, with the turn's
 * info ("Done in 3s · 11:47") as the subtitle. User: Copy · Edit & resend · Quote ·
 * Select text. "Select text" turns native selection on for that message (long-press
 * itself opens this sheet, so it can't also start a selection).
 */
function PhoneMessageSheet(props: { ctx: TimelineCtx; state: NonNullable<ReturnType<typeof currentMessageMenu>> }) {
  const target = () => props.state.target;
  const turn = () => props.ctx.session.turns().find((t) => t.turnId === target().turnId);
  const copy = (text: string) => void copyWithFallback(text).then((ok) => showActionNotice(ok ? 'Copied' : "Couldn't copy the message."));
  const info = () => {
    const t = turn();
    if (!t || target().role !== 'assistant') return undefined;
    const done = t.stopped ? 'Stopped' : t.error ? 'Failed' : t.durationMs !== undefined ? `Done in ${formatDuration(t.durationMs)}` : t.live ? 'Working…' : 'Done';
    const cost = t.costUsd !== undefined && t.costUsd >= 0.0005 ? `$${t.costUsd.toFixed(t.costUsd < 0.1 ? 3 : 2)}` : undefined;
    const at = turnTime(t);
    return [done, at !== undefined ? new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : undefined, cost].filter(Boolean).join(' · ');
  };
  const selectText = () => {
    const el = document.querySelector(`[data-testid="tl-turn-${CSS.escape(target().turnId)}"]`) as HTMLElement | null;
    const root = el;
    root?.setAttribute('data-select-text', '');
    showActionNotice('Text selection is on for this message');
  };
  const sections = (): ActionItem[][] => {
    const t = target();
    const s = props.ctx.session;
    if (t.role === 'user') {
      return [
        [
          { id: 'copy', label: 'Copy', icon: Copy, onSelect: () => copy(t.text) },
          ...(t.entryId
            ? [
                {
                  id: 'edit',
                  label: 'Edit & resend',
                  icon: Pencil,
                  onSelect: () => {
                    const entry = s.state.entries.find((e) => e.id === t.entryId);
                    const tr = turn();
                    if (entry && tr) void requestEdit(props.ctx, tr, entry, props.state.anchor);
                  },
                },
              ]
            : []),
          { id: 'quote', label: 'Quote', icon: Quote, onSelect: () => quoteIntoComposer(s.key, t.text) },
        ],
        [{ id: 'select', label: 'Select text', icon: TextSelect, onSelect: selectText }],
      ];
    }
    const tr = turn();
    return [
      [
        { id: 'copy', label: 'Copy', icon: Copy, onSelect: () => copy(plainOfMarkdown(t.text)) },
        { id: 'copy-md', label: 'Copy as Markdown', icon: FileCode, onSelect: () => copy(t.text) },
        { id: 'quote', label: 'Quote in reply', icon: Quote, onSelect: () => quoteIntoComposer(s.key, t.text) },
      ],
      [
        ...(t.userEntryId
          ? [
              {
                id: 'fork',
                label: 'Fork from here',
                icon: SquareStack,
                onSelect: () => void forkFrom(props.ctx.hostId, s.sessionId(), t.userEntryId, props.ctx.openSession).catch((error) => showActionNotice(`Couldn't fork: ${actionError(error)}`)),
              },
            ]
          : []),
        ...(tr && tr.kind === 'turn' && !tr.live
          ? [
              {
                id: 'retry',
                label: 'Retry',
                icon: RotateCcw,
                disabled: !!s.state.run.running,
                onSelect: () => void retryTurn(s, tr).catch((error) => showActionNotice(`Couldn't retry: ${actionError(error)}`)),
              },
            ]
          : []),
        { id: 'share', label: 'Share', icon: Share2, onSelect: () => tr && void shareTurn(tr).catch((error) => showActionNotice(`Couldn't share: ${actionError(error)}`)) },
        { id: 'select', label: 'Select text', icon: TextSelect, onSelect: selectText },
      ],
    ];
  };
  haptic('impact');
  return (
    <ActionSheet
      open
      onClose={closeMessageMenu}
      title={target().role === 'user' ? 'Your message' : 'Reply'}
      subtitle={info()}
      sections={sections()}
      testId="message-sheet"
    />
  );
}

/**
 * Edit on a user message: a direct tree jump when nothing would be lost and
 * the file-summary option is absent; otherwise a small confirm menu (replace
 * the draft / also undo file changes).
 */
export async function requestEdit(ctx: TimelineCtx, turn: Turn, entry: SessionEntry, anchor: DOMRect): Promise<void> {
  const draft = composerDraft(ctx.session.key);
  const canUndo = !!ctx.undoFiles?.canUndo(turn.turnId);
  const guarded = !!draft.text.trim() || draft.images.length > 0;
  if (!canUndo && !guarded) {
    try {
      await editMessage(ctx.session, entry);
    } catch (error) {
      showActionNotice(`Couldn't edit: ${actionError(error)}`);
    }
    return;
  }
  openEditRequest({
    anchor,
    turnId: turn.turnId,
    entryId: entry.id,
    text: messageText(entry),
    images: entryImages(entry),
    canUndo,
  });
}

/** The one message menu + edit menu of a timeline (portaled). */
export function TurnMenus(props: { ctx: TimelineCtx }) {
  const [alsoUndo, setAlsoUndo] = createSignal(false);
  createEffect(on(currentEditRequest, () => setAlsoUndo(false)));

  const confirmEdit = async (request: NonNullable<ReturnType<typeof currentEditRequest>>) => {
    // Read the toggle before closing: closing resets it through the
    // `currentEditRequest` effect (Z-28 "Also undo file changes").
    const withUndo = request.canUndo && alsoUndo();
    closeEditRequest();
    try {
      if (withUndo) await props.ctx.undoFiles?.undoFrom(request.turnId);
      await treeJump(props.ctx.session, request.entryId);
      restoreComposer(props.ctx.session.key, { text: request.text, images: request.images });
      focusComposer(props.ctx.session.key);
    } catch (error) {
      showActionNotice(`Couldn't edit: ${actionError(error)}`);
    }
  };

  const menu = currentMessageMenu;
  return (
    <>
      <Show when={!layout().desktop() && menu()} keyed>
        {(state) => <PhoneMessageSheet ctx={props.ctx} state={state} />}
      </Show>
      <Show when={layout().desktop() && menu()} keyed>
        {(state) => (
          <AnchoredMenu anchor={state.anchor} onDismiss={closeMessageMenu} label="Message actions" width={224}>
            <MenuRow
              icon={<Copy size={18} />}
              label="Copy"
              onSelect={() => {
                closeMessageMenu();
                void copyWithFallback(state.target.text).then((ok) => !ok && showActionNotice("Couldn't copy the message."));
              }}
            />
            <MenuRow
              icon={<Quote size={18} />}
              label="Quote"
              onSelect={() => {
                closeMessageMenu();
                quoteIntoComposer(props.ctx.session.key, state.target.text);
              }}
            />
            <Show when={state.target.userEntryId}>
              {(entryId) => (
                <MenuRow
                  icon={<SquareStack size={18} />}
                  label="Fork from here"
                  onSelect={() => {
                    closeMessageMenu();
                    void forkFrom(props.ctx.hostId, props.ctx.session.sessionId(), entryId(), props.ctx.openSession).catch((error) =>
                      showActionNotice(`Couldn't fork: ${actionError(error)}`),
                    );
                  }}
                />
              )}
            </Show>
            <Show when={state.target.role === 'user' && state.target.entryId}>
              {(entryId) => (
                <MenuRow
                  icon={<Pencil size={18} />}
                  label="Edit message"
                  onSelect={() => {
                    const entry = props.ctx.session.state.entries.find((e) => e.id === entryId());
                    const turn = props.ctx.session.turns().find((t) => t.turnId === state.target.turnId);
                    closeMessageMenu();
                    if (entry && turn) void requestEdit(props.ctx, turn, entry, state.anchor);
                  }}
                />
              )}
            </Show>
          </AnchoredMenu>
        )}
      </Show>
      <Show when={currentEditRequest()} keyed>
        {(request) => (
          <AnchoredMenu anchor={request.anchor} onDismiss={closeEditRequest} label="Edit message" width={260}>
            <Show when={!request.canUndo && (composerDraft(props.ctx.session.key).text.trim() || composerDraft(props.ctx.session.key).images.length)}>
              <MenuLabel>Replace your draft?</MenuLabel>
            </Show>
            <Show when={request.canUndo}>
              <MenuRow
                icon={alsoUndo() ? <Check size={18} /> : <span class="tl-menu-check" />}
                label="Also undo file changes"
                onSelect={() => setAlsoUndo(!alsoUndo())}
              />
            </Show>
            <MenuRow icon={<Pencil size={18} />} label="Edit message" onSelect={() => void confirmEdit(request)} />
            <MenuRow label="Cancel" onSelect={closeEditRequest} />
          </AnchoredMenu>
        )}
      </Show>
    </>
  );
}
