/**
 * The questions in a reply (phone spec §3.4, PH-21) — the app side of the
 * utility extension's `/answer` (`extract_questions` / `compose_answers` in
 * extensions/unipi/utility/src/answer/extract.rs): paragraphs (a wrapped
 * question stays whole; list items and headings start their own), code fences
 * skipped, every sentence ending in `?` is one question. A list right under a
 * question (items without `?`) becomes its choices.
 */

export type Question = { text: string; options: string[] };

const MAX_QUESTIONS = 12;
const MARKER = /^\s*(?:[-*+]\s+|\d+[.)]\s+|[a-z][.)]\s+|#{1,6}\s+|>\s*)/i;
const clean = (s: string) =>
  s
    .replace(MARKER, '')
    .replace(/\*\*|__|`/g, '')
    .replace(/\s+/g, ' ')
    .trim();

/** Sentences ending in `?` ("e.g." / "v1.2" don't end a sentence). */
function questionSentences(text: string): string[] {
  const out: string[] = [];
  const re = /[^.!?]*(?:[.!](?=\S)[^.!?]*)*\?+/g;
  for (const m of text.matchAll(re)) {
    const q = m[0].replace(/^[\s.!]+/, '').trim();
    if (q.length >= 4) out.push(q);
  }
  return out;
}

export function extractQuestions(reply: string): Question[] {
  const paras: Array<{ text: string; item: boolean }> = [];
  let cur: string[] = [];
  let item = false;
  let fenced = false;
  const flush = () => {
    if (cur.length) paras.push({ text: cur.join(' '), item });
    cur = [];
    item = false;
  };
  for (const raw of reply.split('\n')) {
    const t = raw.trimStart();
    if (t.startsWith('```') || t.startsWith('~~~')) {
      flush();
      fenced = !fenced;
      continue;
    }
    if (fenced) continue;
    if (!raw.trim()) {
      flush();
      continue;
    }
    if (MARKER.test(raw)) {
      flush();
      item = /^\s*(?:[-*+]|\d+[.)]|[a-z][.)])\s+/i.test(raw);
    }
    cur.push(raw);
  }
  flush();

  const out: Question[] = [];
  for (let i = 0; i < paras.length; i += 1) {
    const text = clean(paras[i]!.text);
    if (!text.includes('?')) continue;
    const qs = questionSentences(text);
    if (!qs.length) continue;
    // Choices: the list items right after the (last) question of this paragraph.
    const options: string[] = [];
    for (let j = i + 1; j < paras.length && paras[j]!.item && !paras[j]!.text.includes('?') && options.length < 6; j += 1) options.push(clean(paras[j]!.text));
    qs.forEach((q, k) => {
      if (out.some((o) => o.text === q) || out.length >= MAX_QUESTIONS) return;
      out.push({ text: q, options: k === qs.length - 1 ? options : [] });
    });
  }
  return out;
}

/** The message the answers send (the extension's `compose_answers` format). */
export function composeAnswers(questions: readonly Question[], answers: readonly string[], note = ''): string | undefined {
  const parts: string[] = [];
  const skipped: string[] = [];
  questions.forEach((q, i) => {
    const a = (answers[i] ?? '').trim();
    if (a) parts.push(`${i + 1}. > ${q.text}\n\n${a}`);
    else skipped.push(`#${i + 1}`);
  });
  const n = note.trim();
  if (!parts.length && !n) return undefined;
  const lines = [parts.join('\n\n')].filter(Boolean);
  if (n) lines.push(n);
  if (skipped.length && parts.length) lines.push(`(Not answered: ${skipped.join(', ')})`);
  return lines.join('\n\n');
}
