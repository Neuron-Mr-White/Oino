/**
 * [ask-user] The bottom dock's "question waiting" chip: `unipi.ask-user` questions are answered
 * in their inline timeline card (`AskUserCard`), so the dock no longer shows the form — only
 * this small sticky chip above the composer, which scrolls the card into view. Nothing opens or
 * scrolls by itself (owner 2026-11-04: no jump to a separate "Answer" page on opening a chat).
 */
import { Show } from 'solid-js';
import { MessageCircleQuestion } from 'lucide-solid';
import { askDialogFor, askDialogQuestions, isAskDialog } from '../../lib/chat/askUser';
import { jumpTimelineToTurn } from '../../lib/find';
import type { Turn } from '../../lib/session/turns';
import type { UiRequest } from '../../ui/types';
import '../timeline/cards/askUser.css';

export type AskCall = { turnId: string; toolId: string; dialog: UiRequest };

/** How far back to look for a waiting call (it is in the live tail turn in practice). */
const SCAN_TURNS = 6;

/** The ask dialogs whose call is in the loaded turns (those render inline); the rest stay docked. */
export function inlineAskCalls(dialogs: readonly UiRequest[], turns: readonly Turn[]): AskCall[] {
  const asks = dialogs.filter(isAskDialog);
  if (!asks.length) return [];
  const out: AskCall[] = [];
  for (let i = turns.length - 1, n = 0; i >= 0 && n < SCAN_TURNS && out.length < asks.length; i -= 1, n += 1) {
    for (const seg of turns[i]!.segments) {
      const tool = seg.ask;
      if (!tool || (tool.state !== 'running' && tool.state !== 'preparing')) continue;
      const d = askDialogFor(asks, tool);
      if (d && !out.some((c) => c.dialog.id === d.id)) out.push({ turnId: turns[i]!.turnId, toolId: tool.id, dialog: d });
    }
  }
  return out;
}

/** Scrolls the card of `call` into view (mounting its turn first) and flashes it. */
export function revealAskCard(call: Pick<AskCall, 'turnId' | 'toolId'>): void {
  jumpTimelineToTurn(call.turnId);
  const find = () => document.querySelector<HTMLElement>(`[data-ask-call="${CSS.escape(call.toolId)}"]`);
  let tries = 0;
  const step = () => {
    const el = find();
    if (!el) {
      if ((tries += 1) < 8) requestAnimationFrame(step);
      return;
    }
    el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    el.removeAttribute('data-flash');
    void el.offsetWidth;
    el.setAttribute('data-flash', '');
    setTimeout(() => el.removeAttribute('data-flash'), 950);
    el.querySelector<HTMLElement>('.ask-opt:not(:disabled), .ask-other')?.focus({ preventScroll: true });
  };
  requestAnimationFrame(step);
}

export function AskPendingChip(props: { calls: readonly AskCall[] }) {
  const first = () => props.calls[0];
  const count = () => props.calls.reduce((n, c) => n + Math.max(1, askDialogQuestions(c.dialog).length), 0);
  const text = () => {
    const qs = (first() ? askDialogQuestions(first()!.dialog) : []);
    if (props.calls.length === 1 && qs.length === 1) return qs[0]!.question;
    return `${count()} questions waiting`;
  };
  return (
    <Show when={first()}>
      {(c) => (
        <button type="button" class="ask-pending" onClick={() => revealAskCard(c())} title="Show the question in the chat" data-testid="ask-pending">
          <MessageCircleQuestion size={15} aria-hidden="true" />
          <span class="ask-pending-text">{text()}</span>
          <span class="ask-pending-go">Answer ↑</span>
        </button>
      )}
    </Show>
  );
}
