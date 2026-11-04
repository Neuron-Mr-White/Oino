/**
 * The working line above the composer: the one obvious place that says the chat is running
 * (or waits on you). Replaces the "Subagents N running · Open" pill under the composer and
 * the "question waiting · Answer ↑" chip (owner 2026-11-04).
 *
 * Height eases in/out (`.wl-shell`, a grid-rows reveal) so the dock growing never jumps the
 * chat; the timeline observes the dock and keeps a pinned chat pinned. The sweep uses the same
 * `.shimmer` as the in-chat "Working for …" title; reduced motion keeps a static accent dot.
 */
import { Show, createMemo, createSignal, createEffect, on } from 'solid-js';
import { ChevronRight, MessageCircleQuestion } from 'lucide-solid';
import { useClock } from '../../lib/clock';
import { formatDuration } from '../../lib/chat/model';
import { createPresence } from '../../lib/motion';
import { workingLineOf, type WorkingLineModel } from '../../lib/status/workingLine';
import { askQuestions } from '../../lib/chat/askUser';
import { workingMessageOf } from '../../ui/slots/effects';
import { openSessionModal } from '../session/modalTarget';
import { revealAskCard, type AskCall } from './AskPendingChip';
import type { SessionHandle } from '../../lib/session/store';
import './workingLine.css';

function askText(calls: readonly AskCall[]): string | undefined {
  const first = calls[0];
  if (!first) return undefined;
  const count = calls.reduce((n, c) => n + Math.max(1, askQuestions(c.dialog.questions).length), 0);
  const qs = askQuestions(first.dialog.questions);
  return calls.length === 1 && qs.length === 1 ? qs[0]!.question : `${count} questions waiting`;
}

export function WorkingLine(props: { session: SessionHandle; askCalls: readonly AskCall[]; tray?: string }) {
  const model = createMemo<WorkingLineModel | undefined>(() =>
    workingLineOf({
      run: props.session.state.run,
      ask: askText(props.askCalls),
      tray: props.tray,
      message: workingMessageOf(props.session.key),
    }),
  );
  // Keep the last model while the shell eases out, so the text does not blank mid-collapse.
  const [shown, setShown] = createSignal<WorkingLineModel | undefined>(undefined);
  createEffect(on(model, (m) => m && setShown(m)));
  const presence = createPresence(() => !!model());
  const m = () => model() ?? shown();
  const working = () => m()?.kind === 'working';
  // A working episode without a host start time counts from when the line appeared.
  let appeared = Date.now();
  createEffect(on(() => !!model(), (on_) => on_ && (appeared = Date.now())));
  const now = useClock(() => working() && !!model());
  const elapsed = () => formatDuration(Math.max(0, now() - (m()?.since ?? appeared)));
  const open = () => openSessionModal({ hostId: props.session.hostId, sessionId: props.session.sessionId(), tab: 'running' });
  return (
    <Show when={presence.mounted()}>
      <div class="wl-shell" data-leaving={presence.leaving() ? '' : undefined}>
        <div class="wl-clip">
          <Show when={m()}>
            {(line) => (
              <div class="wl" data-kind={line().kind} data-testid="working-line" role="status" aria-live="off">
                <Show
                  when={line().kind === 'needs-you'}
                  fallback={
                    <>
                      <span class="wl-dot" aria-hidden="true" />
                      <span class="wl-text" classList={{ shimmer: line().kind === 'working' }} data-testid="working-line-text">{line().text}</span>
                      <Show when={line().kind === 'working'}>
                        <span class="wl-time" data-testid="working-line-time">{elapsed()}</span>
                      </Show>
                      <Show when={line().kind === 'background' || line().openRunning || props.tray}>
                        <button type="button" class="wl-open" onClick={open} title="Open the Running tab" data-testid="work-tray-button">
                          Open <ChevronRight size={13} />
                        </button>
                      </Show>
                    </>
                  }
                >
                  <button
                    type="button"
                    class="wl-ask"
                    onClick={() => props.askCalls[0] && revealAskCard(props.askCalls[0])}
                    title="Show the question in the chat"
                    data-testid="ask-pending"
                  >
                    <MessageCircleQuestion size={15} aria-hidden="true" />
                    <span class="wl-text">{line().text}</span>
                    <span class="wl-go">Needs you · Answer ↑</span>
                  </button>
                </Show>
              </div>
            )}
          </Show>
        </div>
      </div>
    </Show>
  );
}
