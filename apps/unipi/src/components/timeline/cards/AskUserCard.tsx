/**
 * [ask-user] The `ask_user` question card, inline in the timeline at the point the agent asked
 * (owner 2026-11-04: "Ask user should stay in the chat nicely, not under an accordion"; no
 * separate page, no auto-navigation). `turns.ts` lifts every `ask_user` call out of the
 * "Worked for …" fold into `Segment.ask`; `TurnRow` renders it after the segment's prose.
 *
 *   waiting   question(s) · options as buttons (checkboxes for multi-select) · "Other" field ·
 *             Skip / Submit. Answers go straight back on the session stream (the dialog the
 *             `unipi.ask-user` form opened); the bottom dock only shows a small "question
 *             waiting" chip that scrolls here (`AskPendingChip`).
 *   settled   one compact "You answered: …" row that stays in place; it expands to the Q&A.
 *
 * Phone: the same card inline; the expand button opens it in a full sheet, only on a tap.
 */
import { createEffect, createMemo, createSignal, For, Index, onCleanup, Show } from 'solid-js';
import { Check, ChevronRight, Maximize2, MessageCircleQuestion } from 'lucide-solid';
import { layout } from '../../../lib/layout';
import { expansionOpen, toggleExpansion } from '../../../lib/timeline/expansion';
import { renderChatMarkdown } from '../../../lib/chat/markdown';
import { answerWords, askAnswers, askCallQuestions, askDialogFor, askDialogQuestions, askResponse, askSettled, isAnswered, skipResponse, type AskAnswer, type AskQ, type AskSettled } from '../../../lib/chat/askUser';
import type { ToolRun } from '../../../lib/session/turns';
import { sessionUi } from '../../../ui/registries';
import { dialogResponse } from '../../../ui/dialogs/adapter';
import { registerPendingAsk } from '../../../lib/chat/askPending';
import { Sheet } from '../../phone';
import type { TimelineCtx } from '../context';
import './askUser.css';

const EMPTY: AskAnswer = { selected: [], skipped: true };

/** Question text as chat prose: the same markdown renderer and typography as an assistant reply (lists, `code`, **bold**, line breaks). */
function QuestionText(props: { text: string }) {
  return <div class="ask-q chat-prose md" data-testid="ask-q" innerHTML={renderChatMarkdown(props.text)} />;
}

export function AskUserCard(props: { tool: ToolRun; ctx: TimelineCtx }) {
  const registry = () => sessionUi(props.ctx.hostId, props.ctx.sessionId());
  const dialog = createMemo(() => {
    const t = props.tool;
    if (t.state !== 'running' && t.state !== 'preparing') return undefined;
    return askDialogFor(registry().dialogs(), t);
  });
  const questions = createMemo<AskQ[]>(() => {
    const d = dialog();
    if (d) {
      const qs = askDialogQuestions(d);
      if (qs.length) return qs;
    }
    const det = askCallQuestions((props.tool.details ?? {}) as Record<string, unknown>);
    return det.length ? det : askCallQuestions(props.tool.args);
  });
  const [answers, setAnswers] = createSignal<AskAnswer[]>([]);
  /** Sent from here: the card settles at once (the result follows on the stream). */
  const [sent, setSent] = createSignal<AskSettled | undefined>();
  /** What the recap lists: the answers sent from here, else the result's. */
  const recap = () => (sent() ? answers() : (askAnswers(props.tool, questions()) ?? []));
  const [sheet, setSheet] = createSignal(false);
  const phone = () => !layout().desktop();
  const waiting = () => (props.tool.state === 'running' || props.tool.state === 'preparing') && !sent();
  const settled = createMemo<AskSettled | undefined>(() => sent() ?? (waiting() ? undefined : askSettled(props.tool, questions())));

  const at = (i: number) => answers()[i] ?? EMPTY;
  const put = (i: number, next: AskAnswer) =>
    setAnswers((list) => {
      const out = [...list];
      while (out.length <= i) out.push(EMPTY);
      out[i] = { ...next, skipped: !isAnswered(next) };
      return out;
    });
  const pick = (i: number, q: AskQ, value: string) => {
    const cur = at(i);
    if (q.multi) {
      const has = cur.selected.includes(value);
      put(i, { ...cur, selected: has ? cur.selected.filter((v) => v !== value) : [...cur.selected, value] });
    } else {
      // One choice: picking an option replaces a typed answer (and picking it again clears it).
      put(i, { selected: cur.selected[0] === value ? [] : [value], skipped: false });
    }
  };
  const type = (i: number, q: AskQ, text: string) => {
    const cur = at(i);
    put(i, { selected: q.multi ? cur.selected : [], ...(text ? { custom_text: text } : {}), skipped: false });
  };
  const any = () => questions().some((_, i) => isAnswered(at(i)));

  const send = (value: unknown, local: AskSettled) => {
    const d = dialog();
    if (!d) return;
    registry().dismissDialog(d.id);
    setSent(local);
    setSheet(false);
    void props.ctx.session.cmd(dialogResponse(d.id, value)).catch((error: unknown) => {
      console.warn('[ask-user] answer failed', d.id, error);
      setSent(undefined);
    });
  };
  const submit = () => {
    if (!any()) return;
    const qs = questions();
    const list = qs.map((_, i) => at(i));
    const text = qs.length <= 1 ? answerWords(qs[0], list[0]) : qs.map((q, i) => `${q.header || `#${i + 1}`}: ${answerWords(q, list[i])}`).join(' · ');
    // Superagent's ask_user is a plain `input` dialog: the answer is the typed text itself.
    send(dialog()?.method === 'input' ? (list[0]?.custom_text?.trim() ?? '') : askResponse(qs, list), { tone: 'ok', label: 'You answered', text });
  };
  const skip = () => send(dialog()?.method === 'input' ? undefined : skipResponse(questions()), { tone: 'muted', label: 'You skipped the questions', text: '' });
  /** The first question that takes a typed answer: where a composer note goes when nothing was picked. */
  const noteTarget = () => questions().findIndex((q) => q.other);
  // [owner-fix-9] While this card waits, a composer send submits it first (the typed text
  // follows as a normal message): picks as they are, nothing picked = skipped.
  createEffect(() => {
    if (!waiting() || !dialog()) return;
    const key = props.ctx.session.key;
    onCleanup(
      registerPendingAsk(key, {
        id: props.tool.id,
        submit: (note) => {
          if (any()) return void submit();
          // Nothing picked and the user typed in the composer: that text IS the answer
          // (the "Other" answer of the first question), not a skip followed by a message.
          const target = noteTarget();
          const text = note?.trim();
          if (text && target >= 0) {
            type(target, questions()[target]!, text);
            submit();
            return true;
          }
          skip();
        },
      }),
    );
  });

  const expandKey = () => `ask:${props.tool.id}`;
  const open = () => expansionOpen(props.ctx.sessionId(), expandKey(), false);

  const Form = (fp: { inSheet?: boolean }) => (
    <div
      class="ask-form"
      data-sheet={fp.inSheet ? '' : undefined}
      // Esc inside the card must not stop the run that waits for this answer (Z-55).
      on:keydown={(e: KeyboardEvent) => {
        if (e.key === 'Escape' && !e.defaultPrevented) {
          e.preventDefault();
          (e.target as HTMLElement | null)?.blur?.();
        }
        if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !e.defaultPrevented) {
          e.preventDefault();
          submit();
        }
      }}
    >
      <Index each={questions()}>
        {(q, i) => (
          <section class="ask-item" data-testid="ask-question">
            <Show when={q().header && questions().length > 1}>
              <span class="ask-chip">{q().header}</span>
            </Show>
            <QuestionText text={q().question} />
            <Show when={q().options.length > 0}>
              <div class="ask-options" role={q().multi ? 'group' : 'radiogroup'} aria-label={q().question}>
                <For each={q().options}>
                  {(o, k) => {
                    const on = () => at(i).selected.includes(o.value);
                    return (
                      <button
                        type="button"
                        class="ask-opt"
                        role={q().multi ? 'checkbox' : 'radio'}
                        aria-checked={on()}
                        disabled={!dialog()}
                        onClick={() => pick(i, q(), o.value)}
                        data-testid="ask-option"
                      >
                        <span class="ask-mark" data-multi={q().multi ? '' : undefined} data-on={on() ? '' : undefined} aria-hidden="true">
                          <Show when={q().multi} fallback={<span class="ask-num">{k() + 1}</span>}>
                            <Show when={on()}>
                              <Check size={12} strokeWidth={3} />
                            </Show>
                          </Show>
                        </span>
                        <span class="ask-opt-text">
                          <span class="ask-opt-label">{o.label}</span>
                          <Show when={o.description}>
                            <span class="ask-opt-desc md" innerHTML={renderChatMarkdown(o.description!)} />
                          </Show>
                        </span>
                      </button>
                    );
                  }}
                </For>
              </div>
            </Show>
            <Show when={q().other}>
              <textarea
                class="ask-other"
                rows={1}
                placeholder={q().options.length ? 'Other — type your own answer' : 'Your answer'}
                aria-label={`Other answer to: ${q().question}`}
                disabled={!dialog()}
                value={at(i).custom_text ?? ''}
                onInput={(e) => {
                  const el = e.currentTarget;
                  el.style.height = 'auto';
                  el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
                  type(i, q(), el.value);
                }}
                data-testid="ask-other"
              />
            </Show>
          </section>
        )}
      </Index>
      <div class="ask-foot">
        <Show when={!dialog()}>
          <span class="ask-wait" data-testid="ask-wait">
            Opening the question…
          </span>
        </Show>
        <span class="flex-1" />
        <button type="button" class="ask-btn ghost" disabled={!dialog()} onClick={skip} data-testid="ask-skip">
          Skip
        </button>
        <button type="button" class="ask-btn primary" disabled={!dialog() || !any()} onClick={submit} data-testid="ask-submit">
          Submit
        </button>
      </div>
    </div>
  );

  return (
    <div class="ask-card" data-ask-call={props.tool.id} data-state={settled() ? 'settled' : 'waiting'} data-testid="ask-card">
      <Show
        when={settled()}
        fallback={
          <>
            <div class="ask-head">
              <MessageCircleQuestion size={15} class="ask-head-icon" aria-hidden="true" />
              <span class="ask-title">{questions().length > 1 ? `The agent asks ${questions().length} questions` : 'The agent asks'}</span>
              <span class="flex-1" />
              <Show when={phone()}>
                <button type="button" class="ask-expand" aria-label="Open the question full screen" onClick={() => setSheet(true)} data-testid="ask-open-sheet">
                  <Maximize2 size={16} />
                </button>
              </Show>
            </div>
            <Form />
            <Show when={phone()}>
              <Sheet open={sheet()} onClose={() => setSheet(false)} label="Question" title="The agent asks" detents={['full']} initialDetent="full" testId="ask-sheet">
                <Show when={sheet()}>
                  <Form inSheet />
                </Show>
              </Sheet>
            </Show>
          </>
        }
      >
        {(s) => (
          <>
            <button
              type="button"
              class="ask-done"
              data-tone={s().tone}
              aria-expanded={open()}
              onClick={() => toggleExpansion(props.ctx.sessionId(), expandKey(), false)}
              title={questions().map((q) => q.question).join('\n')}
              data-testid="ask-answered"
            >
              <MessageCircleQuestion size={15} class="ask-done-icon" aria-hidden="true" />
              <span class="ask-done-label">{s().text ? `${s().label}:` : s().label}</span>
              <Show when={s().text}>
                <span class="ask-done-text">{s().text}</span>
              </Show>
              <ChevronRight size={14} class="ask-done-chev" data-open={open() ? '' : undefined} aria-hidden="true" />
            </button>
            <Show when={open()}>
              <ol class="ask-recap" data-testid="ask-recap">
                <Index each={questions()}>
                  {(q, i) => (
                    <li>
                      <QuestionText text={q().question} />
                      <p class="ask-recap-a">{answerWords(q(), recap()[i])}</p>
                    </li>
                  )}
                </Index>
              </ol>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
