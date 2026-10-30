/**
 * Dialogs from pi answered on the phone (UNI-113): they take over the
 * composer slot (deepseek-harness pattern). select / confirm / input /
 * editor and unipi's ask_user (pager, numbered options, multi-select,
 * free text, auto-advance). Whoever answers first (phone or TUI) wins;
 * the card disappears when pi says the dialog closed.
 */
import { createEffect, createMemo, createSignal, For, Index, Match, onCleanup, Show, Switch } from 'solid-js';
import { Check, ChevronLeft, ChevronRight, MessageCircleQuestion, Pencil, X } from 'lucide-solid';
import type { AskQuestion, Dialog } from '../../lib/chat/store';
import { renderChatMarkdown } from '../../lib/chat/markdown';

type Answer = { selected: string[]; custom_text?: string; skipped: boolean };

const optValue = (o: { label: string; value?: string }) => o.value ?? o.label;

/** mm:ss left until `deadline` (ms since epoch); undefined once it's past or there isn't one. */
function timeLeft(deadline: number | undefined, now: number): string | undefined {
  if (!deadline) return undefined;
  const ms = deadline - now;
  if (ms <= 0) return '0:00';
  const s = Math.ceil(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** A dialog's countdown, ticking every second; calls `onExpire` once when the deadline passes. */
function Countdown(props: { deadline: number | undefined; onExpire: () => void }) {
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  let fired = false;
  createEffect(() => {
    if (!props.deadline || fired) return;
    if (now() >= props.deadline) {
      fired = true;
      props.onExpire();
    }
  });
  const left = () => timeLeft(props.deadline, now());
  return (
    <Show when={left()}>
      <span class="dlg-countdown">{left()}</span>
    </Show>
  );
}

/** The question text (context + question folded together) as markdown. */
function QuestionText(props: { text: string | undefined }) {
  let el: HTMLParagraphElement | undefined;
  createEffect(() => {
    if (el) el.innerHTML = renderChatMarkdown(props.text ?? '');
  });
  return <p class="dlg-question md" ref={el} />;
}

function AskUser(props: { dialog: Dialog; onAnswer: (value: unknown) => void; onHaptic: () => void }) {
  const qs = () => (props.dialog.questions ?? []) as AskQuestion[];
  const [page, setPage] = createSignal(0);
  const [answers, setAnswers] = createSignal<Answer[]>(qs().map(() => ({ selected: [], skipped: true })));
  const [typing, setTyping] = createSignal<number | null>(null);
  const q = () => qs()[page()];
  const a = () => answers()[page()] ?? { selected: [], skipped: true };
  const last = () => page() >= qs().length - 1;
  const answered = createMemo(() => answers().filter((x) => !x.skipped).length);

  const set = (i: number, next: Answer) => setAnswers((list) => list.map((x, j) => (j === i ? next : x)));
  const submit = () => {
    props.onHaptic();
    props.onAnswer({ type: 'answered', answers: answers() });
  };
  const advance = () => {
    if (last()) submit();
    else setPage(page() + 1);
  };
  const toggle = (value: string) => {
    props.onHaptic();
    const cur = a();
    if (q()?.multi_select) {
      const has = cur.selected.includes(value);
      const selected = has ? cur.selected.filter((v) => v !== value) : [...cur.selected, value];
      set(page(), { ...cur, selected, skipped: selected.length === 0 && !cur.custom_text });
      return;
    }
    set(page(), { selected: [value], skipped: false });
    setTyping(null);
    // An action option (end_turn / new_session) fires right away, like the
    // TUI panel — it doesn't wait for the remaining questions.
    const opt = q()?.options.find((o) => optValue(o) === value);
    if (opt?.action) {
      setTimeout(submit, 160);
      return;
    }
    // Plain single choice moves on by itself.
    setTimeout(advance, 160);
  };

  return (
    <div class="dlg">
      <div class="dlg-head">
        <MessageCircleQuestion size={16} class="dlg-head-icon" />
        <span class="dlg-eyebrow">
          {q()?.header || 'Question'}
          <Show when={qs().length > 1}>
            <span class="dlg-count">
              {' '}
              · {page() + 1}/{qs().length}
            </span>
          </Show>
        </span>
        <Countdown deadline={props.dialog.deadline} onExpire={() => props.onAnswer(null)} />
        <span class="flex-1" />
        <button type="button" class="dlg-x" aria-label="Cancel" onClick={() => props.onAnswer({ type: 'cancel' })}>
          <X size={18} />
        </button>
      </div>
      <QuestionText text={q()?.question} />
      <div class="dlg-options">
        <Index each={q()?.options ?? []}>
          {(o, i) => {
            const picked = () => a().selected.includes(optValue(o()));
            return (
              <button type="button" class="dlg-opt" aria-pressed={picked()} onClick={() => toggle(optValue(o()))}>
                <Show when={q()?.multi_select} fallback={<span class="dlg-num">{i + 1}</span>}>
                  <span class="dlg-check" data-on={picked() ? '' : undefined}>
                    <Show when={picked()}>
                      <Check size={12} strokeWidth={3} />
                    </Show>
                  </span>
                </Show>
                <span class="min-w-0 flex-1">
                  <span class="dlg-opt-label">{o().label}</span>
                  <Show when={o().description}>
                    <span class="dlg-opt-desc">{o().description}</span>
                  </Show>
                </span>
              </button>
            );
          }}
        </Index>
        <Show when={q()?.other !== false}>
          <Show
            when={typing() === page()}
            fallback={
              <button type="button" class="dlg-opt other" onClick={() => setTyping(page())}>
                <span class="dlg-num">
                  <Pencil size={12} />
                </span>
                <span class="dlg-opt-label">{a().custom_text ? a().custom_text : 'Type your own answer'}</span>
              </button>
            }
          >
            <textarea
              class="dlg-text"
              rows={2}
              placeholder="Your answer"
              value={a().custom_text ?? ''}
              ref={(el) => queueMicrotask(() => el.focus())}
              onInput={(e) => {
                const text = e.currentTarget.value;
                const cur = a();
                set(page(), { selected: q()?.multi_select ? cur.selected : [], custom_text: text, skipped: !text.trim() && cur.selected.length === 0 });
              }}
            />
          </Show>
        </Show>
      </div>
      <div class="dlg-foot">
        <Show when={qs().length > 1}>
          <button type="button" class="dlg-page" aria-label="Previous question" disabled={page() === 0} onClick={() => setPage(page() - 1)}>
            <ChevronLeft size={18} />
          </button>
          <button type="button" class="dlg-page" aria-label="Next question" disabled={last()} onClick={() => setPage(page() + 1)}>
            <ChevronRight size={18} />
          </button>
        </Show>
        <span class="flex-1" />
        <Show when={!last()}>
          <button type="button" class="dlg-btn ghost" onClick={advance}>
            {a().skipped ? 'Skip' : 'Next'}
          </button>
        </Show>
        <Show when={last()}>
          <button type="button" class="dlg-btn primary" disabled={answered() === 0} onClick={submit}>
            Send
          </button>
        </Show>
      </div>
    </div>
  );
}

function Simple(props: { dialog: Dialog; onAnswer: (value: unknown) => void; onHaptic: () => void }) {
  const d = () => props.dialog;
  const [text, setText] = createSignal(d().prefill ?? '');
  return (
    <div class="dlg">
      <div class="dlg-head">
        <MessageCircleQuestion size={16} class="dlg-head-icon" />
        <span class="dlg-eyebrow">pi asks</span>
        <Countdown deadline={d().deadline} onExpire={() => props.onAnswer(null)} />
        <span class="flex-1" />
        <button type="button" class="dlg-x" aria-label="Dismiss" onClick={() => props.onAnswer(null)}>
          <X size={18} />
        </button>
      </div>
      <p class="dlg-question">{d().title}</p>
      <Switch>
        <Match when={d().kind === 'select'}>
          <div class="dlg-options">
            <Index each={d().options ?? []}>
              {(o, i) => (
                <button
                  type="button"
                  class="dlg-opt"
                  onClick={() => {
                    props.onHaptic();
                    props.onAnswer(o());
                  }}
                >
                  <span class="dlg-num">{i + 1}</span>
                  <span class="dlg-opt-label">{o()}</span>
                </button>
              )}
            </Index>
          </div>
        </Match>
        <Match when={d().kind === 'confirm'}>
          <Show when={d().message}>
            <p class="dlg-message">{d().message}</p>
          </Show>
          <div class="dlg-foot">
            <span class="flex-1" />
            <button type="button" class="dlg-btn ghost" onClick={() => props.onAnswer(false)}>
              No
            </button>
            <button
              type="button"
              class="dlg-btn primary"
              onClick={() => {
                props.onHaptic();
                props.onAnswer(true);
              }}
            >
              Yes
            </button>
          </div>
        </Match>
        <Match when={d().kind === 'input' || d().kind === 'editor'}>
          <textarea
            class="dlg-text"
            rows={d().kind === 'editor' ? 6 : 2}
            placeholder={d().placeholder ?? ''}
            value={text()}
            onInput={(e) => setText(e.currentTarget.value)}
          />
          <div class="dlg-foot">
            <span class="flex-1" />
            <button
              type="button"
              class="dlg-btn primary"
              onClick={() => {
                props.onHaptic();
                props.onAnswer(text());
              }}
            >
              Send
            </button>
          </div>
        </Match>
        <Match when={d().kind === 'custom'}>
          <p class="dlg-message">This one can only be answered in the terminal.</p>
        </Match>
      </Switch>
    </div>
  );
}

export function DialogCard(props: { dialog: Dialog; onAnswer: (value: unknown) => void; onHaptic: () => void }) {
  return (
    <Show when={props.dialog.kind === 'ask_user'} fallback={<Simple {...props} />}>
      <AskUser {...props} />
    </Show>
  );
}

export function DialogStack(props: { dialogs: readonly Dialog[]; onAnswer: (id: number, value: unknown) => void; onHaptic: () => void }) {
  const top = () => props.dialogs[0];
  return (
    <Show when={top()} keyed>
      {(d) => <DialogCard dialog={d} onAnswer={(v) => props.onAnswer(d.id, v)} onHaptic={props.onHaptic} />}
    </Show>
  );
}

export { For };
