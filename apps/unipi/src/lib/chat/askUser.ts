/**
 * [ask-user] The `ask_user` tool in the timeline (owner 2026-11-04: "Ask user should stay in
 * the chat nicely, not under an accordion"): pure helpers behind the inline question card.
 *
 * - The questions come from the open dialog (the `unipi.ask-user` `form` request, settings
 *   applied), else the result's `details.questions`, else the call's own arguments.
 * - The open dialog is found by the `toolCallId` the extension puts on its form request;
 *   older extension builds don't, so a form from `unipi.ask-user` whose questions read the
 *   same as the call's is the match.
 * - Answers go back in the form shape `decode` reads (`extensions/unipi/ask-user/src/dialog.rs`):
 *   `{answers: [{id, selected, other}]}`; Skip sends every question skipped (the agent goes on
 *   and is told they were skipped on purpose), never a cancel (that ends the turn).
 */
import type { UiRequest } from '../../ui/types';

export const ASK_EXTENSION = 'unipi.ask-user';

export type AskOpt = { label: string; description?: string; value: string; action?: string };
export type AskQ = { id: string; question: string; header: string; options: AskOpt[]; multi: boolean; other: boolean };
export type AskAnswer = { selected: string[]; custom_text?: string; skipped: boolean };

/** What the settled card says (`tone` colours its icon). */
export type AskSettled = { tone: 'ok' | 'muted' | 'warning'; label: string; text: string };

const str = (v: unknown): string | undefined => (typeof v === 'string' ? v : undefined);
const MAX_QUESTIONS = 4;

/** Superagent's own `ask_user` tool (`superagent_session.rs`): one `question` string, answered in a plain `input` dialog of the `superagent` extension. */
export const SUPERAGENT_EXTENSION = 'superagent';
const textQuestion = (question: string): AskQ => ({ id: 'q0', question, header: '', options: [], multi: false, other: true });

/** The questions of a call's arguments: `questions[]` (unipi.ask-user), else Superagent's single `question` string. */
export function askCallQuestions(args: Record<string, unknown>): AskQ[] {
  const list = askQuestions(args.questions);
  if (list.length) return list;
  const single = str(args.question)?.trim();
  return single ? [textQuestion(single)] : [];
}

/** The questions an open ask dialog carries: a form's list, or a Superagent input dialog's title. */
export function askDialogQuestions(d: UiRequest): AskQ[] {
  if (d.method === 'input') {
    const title = str(d.title)?.trim();
    return title ? [textQuestion(title)] : [];
  }
  return askQuestions(d.questions);
}

/** Questions from any of the three shapes (form request, result details, call args). */
export function askQuestions(raw: unknown): AskQ[] {
  if (!Array.isArray(raw)) return [];
  const out: AskQ[] = [];
  raw.forEach((q, i) => {
    if (!q || typeof q !== 'object' || out.length >= MAX_QUESTIONS) return;
    const r = q as Record<string, unknown>;
    const question = (str(r.question) ?? '').trim();
    if (!question) return;
    const options: AskOpt[] = Array.isArray(r.options)
      ? r.options.flatMap((o) => {
          if (typeof o === 'string') return o.trim() ? [{ label: o.trim(), value: o.trim() }] : [];
          const ro = (o ?? {}) as Record<string, unknown>;
          const label = (str(ro.label) ?? '').trim();
          if (!label) return [];
          const description = str(ro.description)?.trim();
          const action = str(ro.action);
          return [{ label, value: str(ro.value) ?? label, ...(description ? { description } : {}), ...(action ? { action } : {}) }];
        })
      : [];
    out.push({
      id: str(r.id) ?? `q${i}`,
      question,
      header: (str(r.header) ?? '').trim(),
      options,
      multi: r.multiSelect === true || r.multi_select === true,
      // `other: false` removes the free-text row; a question without options always has it.
      other: options.length === 0 || r.other !== false,
    });
  });
  return out;
}

/** An `unipi.ask-user` question form, or Superagent's `ask_user` input (rendered inline, never docked). */
export function isAskDialog(d: UiRequest): boolean {
  return (d.extensionId === ASK_EXTENSION && d.method === 'form') || (d.extensionId === SUPERAGENT_EXTENSION && d.method === 'input');
}

const sameQuestions = (a: readonly AskQ[], b: readonly AskQ[]) => a.length === b.length && a.every((q, i) => q.question === b[i]!.question);

/** The open dialog of one `ask_user` call. */
export function askDialogFor(dialogs: readonly UiRequest[], call: { id: string; args: Record<string, unknown> }): UiRequest | undefined {
  const exact = dialogs.find((d) => d.toolCallId === call.id);
  if (exact) return exact;
  const mine = askCallQuestions(call.args);
  return dialogs.find((d) => isAskDialog(d) && typeof d.toolCallId !== 'string' && sameQuestions(askDialogQuestions(d), mine));
}

/** The form answer: every question by id; unanswered ones skipped. */
export function askResponse(questions: readonly AskQ[], answers: readonly (AskAnswer | undefined)[]): { answers: Array<{ id: string; selected: string[]; other?: string }> } {
  return {
    answers: questions.map((q, i) => {
      const a = answers[i];
      const other = a?.custom_text?.trim();
      return { id: q.id, selected: a && !a.skipped ? a.selected : [], ...(other ? { other } : {}) };
    }),
  };
}

/** Every question skipped. */
export function skipResponse(questions: readonly AskQ[]): { answers: Array<{ id: string; selected: string[] }> } {
  return { answers: questions.map((q) => ({ id: q.id, selected: [] })) };
}

export function isAnswered(a: AskAnswer | undefined): boolean {
  return !!a && (a.selected.length > 0 || !!a.custom_text?.trim());
}

/** "Postgres, Auth" — one answer in words (labels, not values). */
export function answerWords(q: AskQ | undefined, a: AskAnswer | undefined): string {
  if (!a || !isAnswered(a)) return 'skipped';
  const parts = a.selected.map((v) => q?.options.find((o) => o.value === v)?.label ?? v);
  const t = a.custom_text?.trim();
  if (t) parts.push(t);
  return parts.join(', ');
}

/** Superagent's result text: `{"answer": "…"}`, `{"cancelled": true}` — no per-question keys, no `skipped` flag. */
function superagentAnswer(body: Record<string, unknown>, questions: readonly AskQ[]): AskAnswer[] | undefined {
  if (typeof body.answer === 'string') {
    const text = body.answer.trim();
    return questions.map((_, i) => (i === 0 && text ? { selected: [], custom_text: text, skipped: false } : { selected: [], skipped: true }));
  }
  return undefined; // `cancelled` stays unread: askSettled words it "dismissed".
}

/** Answers from the result text (`User answered your questions:\n{json}`) when there are no details. */
function answersFromOutput(output: string | undefined, questions: readonly AskQ[]): AskAnswer[] | undefined {
  if (!output) return undefined;
  const at = output.indexOf('{');
  const end = output.lastIndexOf('}');
  if (at < 0 || end <= at) return undefined;
  try {
    const parsed = JSON.parse(output.slice(at, end + 1)) as Record<string, unknown>;
    const own = superagentAnswer(parsed, questions);
    if (own) return own;
    const body = parsed as Record<string, { selected?: unknown; custom_text?: unknown; skipped?: unknown }>;
    // Nothing keyed by these questions (a shape this reader does not know): no answers, never "skipped".
    if (!questions.some((q) => q.question in body)) return undefined;
    return questions.map((q) => {
      const a = body[q.question];
      const selected = Array.isArray(a?.selected) ? a.selected.filter((s): s is string => typeof s === 'string') : [];
      const custom = str(a?.custom_text);
      return { selected, ...(custom ? { custom_text: custom } : {}), skipped: a?.skipped === true || (!selected.length && !custom) };
    });
  } catch {
    return undefined;
  }
}

const firstLine = (s: string | undefined) => (s ?? '').split('\n').find((l) => l.trim())?.trim() ?? '';

/** The settled card's one line, from the result (details first, then the text). */
/** The answers a settled call carries (details first, then the result text). */
export function askAnswers(tool: { output?: string; details?: unknown }, questions: readonly AskQ[]): AskAnswer[] | undefined {
  const d = (tool.details ?? {}) as { answers?: unknown };
  return Array.isArray(d.answers) ? (d.answers as AskAnswer[]) : answersFromOutput(tool.output, questions);
}

export function askSettled(tool: { state: string; output?: string; details?: unknown }, questions: readonly AskQ[]): AskSettled {
  const d = (tool.details ?? {}) as { outcome?: unknown; reason?: unknown };
  const answers = askAnswers(tool, questions);
  const words = () => {
    if (!answers) return '';
    if (questions.length <= 1) return answerWords(questions[0], answers[0]);
    return questions.map((q, i) => `${q.header || `#${i + 1}`}: ${answerWords(q, answers[i])}`).join(' · ');
  };
  if (tool.state === 'stopped') return { tone: 'muted', label: 'Not answered', text: 'the run stopped before an answer' };
  if (tool.state === 'error') return { tone: 'warning', label: 'Could not ask', text: firstLine(tool.output) };
  switch (d.outcome) {
    case 'cancelled':
      return { tone: 'muted', label: 'You dismissed the questions', text: '' };
    case 'unavailable':
      return { tone: 'muted', label: 'Not shown', text: d.reason === 'disabled' ? 'Ask user is turned off in settings' : d.reason === 'no-ui' ? 'no app was open to answer' : firstLine(tool.output) };
    case 'clarify':
      return { tone: 'ok', label: 'You asked for more context', text: words() };
    case 'action':
      return { tone: 'ok', label: 'You chose', text: words() };
    default:
      break;
  }
  if (answers?.length && answers.every((a) => !isAnswered(a))) return { tone: 'muted', label: 'You skipped the questions', text: '' };
  if (answers?.length) return { tone: 'ok', label: 'You answered', text: words() };
  if (/cancel/i.test(tool.output ?? '')) return { tone: 'muted', label: 'You dismissed the questions', text: '' };
  return { tone: 'ok', label: 'You answered', text: firstLine(tool.output) };
}
