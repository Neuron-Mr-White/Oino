/**
 * "Answer N questions" (phone spec §3.4, PH-21): on the phone the utility
 * extension's `/answer` hint line ("1 question in the reply — /answer …", a
 * TUI command) becomes a 48 px accent pill docked above the composer. It opens
 * a sheet with each question as a card — its choices as 48 px rows, plus a
 * free-text field — and "Send answers" in the footer, sent as one prompt in
 * the extension's own `/answer` format.
 */
import { createMemo, createSignal, For, Index, Show } from 'solid-js';
import { MessageCircleQuestion } from 'lucide-solid';
import { sessions } from '../../lib/session/store';
import { messageText } from '../../lib/timeline/actions';
import { composeAnswers, extractQuestions } from '../../lib/chat/questions';
import { Button, Sheet } from '../phone';
import './phoneChat.css';
import type { UiEntry } from '../../ui/types';

/** The extension's hint widget (`setWidget` key `unipi-answer-hint`). */
export const isAnswerHint = (entry: UiEntry): boolean => /unipi-answer-hint/.test(entry.id);

/** "3 questions" from the hint's own text, when it says. */
export function hintCount(entry: UiEntry): number | undefined {
  const m = /(\d+)\s+questions?\b/.exec(JSON.stringify(entry.view ?? ''));
  return m ? Number(m[1]) : undefined;
}

export function AnswerQuestions(props: { hostId: string; sessionId: string; entry: UiEntry }) {
  const session = () => sessions().peek(props.hostId, props.sessionId);
  const reply = createMemo(() => {
    const entries = session()?.state.entries ?? [];
    for (let i = entries.length - 1; i >= 0; i -= 1) if (entries[i]!.message?.role === 'assistant') return messageText(entries[i]);
    return '';
  });
  const questions = createMemo(() => extractQuestions(reply()));
  const count = () => questions().length || hintCount(props.entry) || 1;
  const [open, setOpen] = createSignal(false);
  const [answers, setAnswers] = createSignal<string[]>([]);
  const setAnswer = (i: number, value: string) => setAnswers((cur) => Object.assign([...cur], { [i]: value }));
  const message = () => composeAnswers(questions(), answers());
  const send = () => {
    const text = message();
    const s = session();
    if (!text || !s) return;
    s.send(text, { mode: 'prompt' });
    setAnswers([]);
    setOpen(false);
  };
  return (
    <>
      <button type="button" class="ph-answer-pill" onClick={() => setOpen(true)} data-testid="answer-pill">
        <MessageCircleQuestion size={20} aria-hidden="true" />
        <span>{`Answer ${count()} question${count() === 1 ? '' : 's'}`}</span>
      </button>
      <Sheet
        open={open()}
        onClose={() => setOpen(false)}
        label="Answer questions"
        title={`Answer ${count()} question${count() === 1 ? '' : 's'}`}
        detents={['full']}
        initialDetent="full"
        guarded={() => answers().some((a) => !!a?.trim())}
        testId="answer-sheet"
        footer={
          <Button kind="primary" block disabled={!message()} onClick={send} testId="answer-send">
            Send answers
          </Button>
        }
      >
        <div class="ph-answer-list">
          <Index each={questions()}>
            {(q, i) => (
              <section class="ph-answer-card" data-testid="answer-card">
                <p class="ph-answer-q">{q().text}</p>
                <Show when={q().options.length > 0}>
                  <div class="ph-answer-options" role="radiogroup" aria-label={q().text}>
                    <For each={q().options}>
                      {(opt) => (
                        <button type="button" role="radio" aria-checked={answers()[i] === opt} class="ph-answer-option" onClick={() => setAnswer(i, answers()[i] === opt ? '' : opt)}>
                          <span class="ph-mode-radio" aria-hidden="true" />
                          <span>{opt}</span>
                        </button>
                      )}
                    </For>
                  </div>
                </Show>
                <textarea
                  class="ph-answer-field"
                  rows={2}
                  placeholder={q().options.length ? 'Or write your answer' : 'Your answer'}
                  aria-label={`Answer to: ${q().text}`}
                  value={answers()[i] && !q().options.includes(answers()[i]!) ? answers()[i] : ''}
                  onInput={(e) => setAnswer(i, e.currentTarget.value)}
                />
              </section>
            )}
          </Index>
          <Show when={questions().length === 0}>
            <p class="ph-answer-empty">The questions are in the last reply — answer them in a message.</p>
          </Show>
        </div>
      </Sheet>
    </>
  );
}
