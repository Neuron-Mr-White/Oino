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

/** When each session's working line first appeared (an episode without a host start time counts from here). */
const appearedBySession = new Map<string, number>();

function askText(calls: readonly AskCall[]): string | undefined {
  const first = calls[0];
  if (!first) return undefined;
  const count = calls.reduce((n, c) => n + Math.max(1, askQuestions(c.dialog.questions).length), 0);
  const qs = askQuestions(first.dialog.questions);
  return calls.length === 1 && qs.length === 1 ? qs[0]!.question : `${count} questions waiting`;
}

/** Brings a docked dialog (approval / select / …) into view and focuses its first control. */
function revealDockedDialog(from: HTMLElement): void {
  const panel = from.closest('[data-testid=bottom-dock]')?.querySelector<HTMLElement>('[data-testid=dialog-panel], [data-testid=approval-panel]');
  if (!panel) return;
  panel.scrollIntoView({ block: 'nearest' });
  panel.querySelector<HTMLElement>('button, input, textarea, [tabindex]')?.focus();
}

export function WorkingLine(props: { session: SessionHandle; askCalls: readonly AskCall[]; tray?: string; dialog?: string }) {
  const model = createMemo<WorkingLineModel | undefined>(() =>
    workingLineOf({
      run: props.session.state.run,
      ask: askText(props.askCalls),
      tray: props.tray,
      message: workingMessageOf(props.session.key),
      dialog: props.dialog,
    }),
  );
  // Keep the last model while the shell eases out, so the text does not blank mid-collapse.
  const [shown, setShown] = createSignal<WorkingLineModel | undefined>(undefined);
  createEffect(on(model, (m) => m && setShown(m)));
  const presence = createPresence(() => !!model());
  const m = () => model() ?? shown();
  const working = () => m()?.kind === 'working';
  // A working episode without a host start time counts from when the line appeared. `active`
  // is a memo so only a hidden → shown change restarts it: every host state push replaces the
  // run object (a new model), which used to reset the clock to 0s every ~2 s while waiting.
  // The start is kept per session at module level (not in the component): a layout change (side
  // pane open/close, resize) remounts this line, and a local start reset the clock to 0s.
  const key = props.session.key;
  const active = createMemo(() => !!model());
  const appearedAt = (): number => {
    let at = appearedBySession.get(key);
    if (at === undefined) {
      at = Date.now();
      appearedBySession.set(key, at);
    }
    return at;
  };
  createEffect(on(active, (on_) => (on_ ? void appearedAt() : void appearedBySession.delete(key))));
  const now = useClock(() => working() && !!model());
  const elapsed = () => formatDuration(Math.max(0, now() - (m()?.since ?? appearedAt())));
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
                    onClick={(e) => (line().dialog ? revealDockedDialog(e.currentTarget) : props.askCalls[0] && revealAskCard(props.askCalls[0]))}
                    title={line().dialog ? 'Go to the request above' : 'Show the question in the chat'}
                    data-testid={line().dialog ? 'dialog-pending' : 'ask-pending'}
                  >
                    <MessageCircleQuestion size={15} aria-hidden="true" />
                    <span class="wl-text">{line().text}</span>
                    <span class="wl-go">{line().dialog ? 'Needs you ↑' : 'Needs you · Answer ↑'}</span>
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
