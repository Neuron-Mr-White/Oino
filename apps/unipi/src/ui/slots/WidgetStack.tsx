/**
 * `widget` slot host (WP-U10, G-3; ui-protocol §6.2, §3.2 `setWidget` row):
 * the stack above the composer (and `widgetBelow` below it, desktop only) —
 * each entry card-less with a 1 px rule between, `lines` views in the
 * monospace mini style and `tree` views through `EntryView`.
 *
 * Phone (§13.6 row 6): `widgetBelow` folds into this stack, and beyond 3
 * entries only the first 3 show — the rest sit behind a "+N widgets" chip
 * (§6.4 per-slot cap).
 */
import { createSignal, For, Show } from 'solid-js';
import { AnswerQuestions, isAnswerHint } from '../../components/conversation/AnswerQuestions';
import { ChevronDown, ChevronRight, ChevronUp } from 'lucide-solid';
import { EntryView } from '../NodeView';
import { openSessionModal } from '../../components/session/modalTarget';
import { liveTrayEntries, trayButtonText } from './trayHint';
import type { UiEntry } from '../types';
import type { UiTarget } from '../actions';

const PHONE_MAX = 3;

export function WidgetStack(props: { entries: UiEntry[]; target: UiTarget; phone?: boolean; fallbackTrayLabel?: string }) {
  // The `/answer` hint line ("… — /answer", a terminal command) becomes the "Answer N questions"
  // pill on the phone (§3.4, PH-21) and on the desktop ([gui-cleanup]: /answer is terminal-only).
  const answerHint = () => (props.target.sessionId ? props.entries.find(isAnswerHint) : undefined);
  const entries = () => liveTrayEntries(props.entries.filter((e) => !isAnswerHint(e)));
  const hasEntryTray = () => !!props.target.sessionId && entries().some((e) => typeof trayButtonText(e) === 'string');
  const synthLabel = () => (props.target.sessionId && !hasEntryTray() ? props.fallbackTrayLabel : undefined);
  const over = () => (props.phone ? Math.max(0, entries().length - PHONE_MAX) : 0);
  const [expanded, setExpanded] = createSignal(false);
  const shown = () => (over() && !expanded() ? entries().slice(0, PHONE_MAX) : entries());
  return (
    <>
    <Show when={answerHint()} keyed>
      {(hint) => <AnswerQuestions hostId={props.target.hostId} sessionId={props.target.sessionId!} entry={hint} />}
    </Show>
    <Show when={entries().length > 0 || !!synthLabel()}>
      <div class="ui-widgets" data-count={entries().length + (synthLabel() ? 1 : 0)} data-testid="ui-widgets">
        <Show when={synthLabel()}>
          {(label) => (
            <div class="ui-widget" data-entry="work-tray-strip">
              <button
                type="button"
                class="ui-tray-btn"
                onClick={() =>
                  props.target.sessionId &&
                  openSessionModal({ hostId: props.target.hostId, sessionId: props.target.sessionId, tab: 'running' })
                }
                data-testid="work-tray-button"
              >
                <span class="ui-tray-dot" aria-hidden="true" />
                <span class="ui-tray-btn-label">{label()}</span>
                <span class="ui-tray-btn-open">
                  Open <ChevronRight size={13} />
                </span>
              </button>
            </div>
          )}
        </Show>
        <For each={shown()}>
          {(entry) => {
            const trayLabel = () => (props.target.sessionId ? trayButtonText(entry) : undefined);
            return (
              <div class="ui-widget" data-entry={entry.id}>
                <Show
                  when={typeof trayLabel() === 'string' ? trayLabel() : undefined}
                  fallback={
                    <>
                      <Show when={entry.title}>
                        <p class="ui-widget-title">{entry.title}</p>
                      </Show>
                      <EntryView entry={entry} target={props.target} mode="auto" />
                    </>
                  }
                >
                  {(label) => (
                    <button
                      type="button"
                      class="ui-tray-btn"
                      onClick={() =>
                        props.target.sessionId &&
                        openSessionModal({ hostId: props.target.hostId, sessionId: props.target.sessionId, tab: 'running' })
                      }
                      data-testid="work-tray-button"
                    >
                      <span class="ui-tray-dot" aria-hidden="true" />
                      <span class="ui-tray-btn-label">{label()}</span>
                      <span class="ui-tray-btn-open">
                        Open <ChevronRight size={13} />
                      </span>
                    </button>
                  )}
                </Show>
              </div>
            );
          }}
        </For>
        <Show when={over() > 0}>
          <button type="button" class="ui-widgets-more" aria-expanded={expanded()} onClick={() => setExpanded((v) => !v)} data-testid="ui-widgets-more">
            {expanded() ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
            {expanded() ? `Hide ${over()} widgets` : `+${over()} widgets`}
          </button>
        </Show>
      </div>
    </Show>
    </>
  );
}
