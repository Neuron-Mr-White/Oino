/**
 * Phone error card (phone spec §3.4, PH-14 / PH-28): ONE card per failed turn,
 * however many attempts the auto-retry made — the turn's earlier error items
 * render nothing, the last one renders the card:
 *
 *   The model request failed                 (title 16 600)
 *   One-line reason                          (lib/chat/errorCopy)
 *   Failed after 4 attempts  ›               (expands the attempt list)
 *   [Retry]  [Choose a model | Add a provider]
 *
 * No host paths, `/login` or CLI wording reach the phone.
 */
import { createMemo, createSignal, For, Show } from 'solid-js';
import { ChevronRight, CircleAlert, RotateCcw } from 'lucide-solid';
import type { Turn, WorkItem } from '../../lib/session/turns';
import { errorCopy, cleanReason } from '../../lib/chat/errorCopy';
import { actionError, retryTurn, showActionNotice } from '../../lib/timeline/actions';
import { openSettingsAt } from '../../lib/settings/open';
import { openModelPicker } from '../model/target';
import type { TimelineCtx } from './context';
import { Button } from '../phone';
import '../conversation/phoneChat.css';

type ErrorItem = Extract<WorkItem, { kind: 'error' }>;

/** The turn an error item belongs to and every error item of that turn (in order). */
export function turnErrors(turns: readonly Turn[], key: string): { turn: Turn; errors: ErrorItem[] } | undefined {
  for (const turn of turns) {
    const errors: ErrorItem[] = [];
    let mine = false;
    for (const seg of turn.segments) {
      for (const w of seg.work) {
        if (w.kind !== 'error') continue;
        errors.push(w);
        if (w.key === key) mine = true;
      }
    }
    if (mine) return { turn, errors };
  }
  return undefined;
}

export function PhoneErrorCard(props: { item: ErrorItem; ctx: TimelineCtx }) {
  const found = createMemo(() => turnErrors(props.ctx.session.turns(), props.item.key));
  const isLast = () => {
    const f = found();
    return !f || f.errors.at(-1)?.key === props.item.key;
  };
  const attempts = () => found()?.errors.length ?? 1;
  const copy = () => errorCopy(props.item.text);
  const [open, setOpen] = createSignal(false);
  const running = () => !!props.ctx.session.state.run.running;
  const retry = () => {
    const t = found()?.turn;
    if (!t) return;
    void retryTurn(props.ctx.session, t).catch((error) => showActionNotice(`Couldn't retry: ${actionError(error)}`));
  };
  return (
    <Show when={isLast()}>
      <div class="ph-error-card" role="alert" data-testid="tl-error">
        <div class="ph-error-head">
          <CircleAlert size={20} class="ph-error-ic" aria-hidden="true" />
          <div class="ph-error-text">
            <p class="ph-error-title" data-testid="tl-error-title">
              {copy().title}
            </p>
            <p class="ph-error-reason">{copy().reason}</p>
          </div>
        </div>
        <Show when={attempts() > 1}>
          <button type="button" class="ph-error-attempts" aria-expanded={open()} onClick={() => setOpen(!open())} data-testid="tl-error-attempts">
            <span>Failed after {attempts()} attempts</span>
            <ChevronRight size={16} class="ph-error-chev" data-open={open() ? '' : undefined} />
          </button>
          <Show when={open()}>
            <ol class="ph-error-list">
              <For each={found()?.errors ?? []}>{(e) => <li>{cleanReason(e.text) || errorCopy(e.text).title}</li>}</For>
            </ol>
          </Show>
        </Show>
        <div class="ph-error-actions">
          <Button kind="secondary" disabled={running() || !found()} onClick={retry} testId="tl-error-retry">
            <RotateCcw size={18} /> Retry
          </Button>
          <Show when={copy().fix === 'model'}>
            <Button kind="primary" onClick={() => openModelPicker({ hostId: props.ctx.hostId, sessionId: props.ctx.sessionId() })} testId="tl-error-fix">
              Choose a model
            </Button>
          </Show>
          <Show when={copy().fix === 'provider'}>
            <Button kind="primary" onClick={() => openSettingsAt('providers', 'sec-providers')} testId="tl-error-fix">
              Add a provider
            </Button>
          </Show>
        </div>
      </div>
    </Show>
  );
}
