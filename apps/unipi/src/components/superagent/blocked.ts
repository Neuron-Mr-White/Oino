/**
 * A blocked ticket's reason is whatever the lead wrote: sometimes one line, often a whole build
 * report with the owner's question at the very end (OIN-62). The workbench pins the question and
 * keeps the rest behind "Read full report", so this splits the text once, without a renderer.
 */
export type BlockedParts = {
  /** The decision the owner has to make, as Markdown. Never empty for a non-empty reason. */
  ask: string;
  /** The first line of the reason, plain and short: the queue / pager line. Empty when it adds nothing over `ask`. */
  reason: string;
  /** Everything that is not the question, as Markdown. Empty when the reason was just the question. */
  report: string;
};

const LABEL = /^(?:questions?(?: for (?:you|the owner|owner))?|needs? from (?:you|the owner|owner)|decisions?(?: needed| required)?|owner questions?|open questions?|your call)$/;
const SHORT = 600;
const REASON_MAX = 140;

/** `**Need from you:** text`, `QUESTION: text`, `## Need from you`: the label and what follows on that line, or null. */
function marker(line: string): { rest: string } | null {
  const heading = /^\s{0,3}#{1,6}\s+/.test(line);
  const body = line.replace(/^\s{0,3}(?:#{1,6}\s+)?(?:[-*>]\s+)?/, '').trim();
  const bold = /^(\*\*|__)\s*([^*_]+?)\s*[:：]?\s*\1\s*[:：]?\s*(.*)$/.exec(body);
  if (bold) return LABEL.test(bold[2]!.trim().toLowerCase()) ? { rest: bold[3]!.trim() } : null;
  const plain = /^([A-Za-z][A-Za-z ]{2,30}?)\s*[:：]\s*(.*)$/.exec(body);
  if (plain) return LABEL.test(plain[1]!.trim().toLowerCase()) ? { rest: plain[2]!.trim() } : null;
  return heading && LABEL.test(body.replace(/[:：]$/, '').trim().toLowerCase()) ? { rest: '' } : null;
}

/** A line that opens the next section of the report, so the question block ends before it. */
const sectionStart = (line: string) => /^\s{0,3}#{1,6}\s/.test(line) || /^(\*\*|__)[^*_]+[:：]?\1[:：]?\s*$/.test(line.trim());

const plain = (line: string) => line.replace(/^\s{0,3}(?:#{1,6}\s+|[-*>]\s+)*/, '').replace(/[*_`]+/g, '').replace(/\s+/g, ' ').trim();
const clip = (text: string) => (text.length > REASON_MAX ? `${text.slice(0, REASON_MAX - 1).trimEnd()}…` : text);
const join = (lines: string[]) => lines.join('\n').replace(/\n{3,}/g, '\n\n').trim();

export function splitBlocked(input: string | null | undefined): BlockedParts {
  const text = (input ?? '').replace(/\r\n?/g, '\n').trim();
  if (!text) return { ask: '', reason: '', report: '' };
  const lines = text.split('\n');
  const first = clip(plain(lines.find((l) => plain(l)) ?? ''));
  const parts = (ask: string, report: string): BlockedParts => ({ ask, report, reason: report && plain(ask) !== first ? first : '' });

  // 1. A labelled block: the last one wins, the question closes the report.
  let at = -1;
  lines.forEach((l, i) => { if (marker(l)) at = i; });
  if (at >= 0) {
    let end = at + 1;
    while (end < lines.length && !sectionStart(lines[end]!)) end += 1;
    const ask = join([marker(lines[at]!)!.rest, ...lines.slice(at + 1, end)]);
    if (ask) return parts(ask, join([...lines.slice(0, at), ...lines.slice(end)]));
  }

  // 2. A trailing question: the last paragraph (with the list under it), when there is a report before it.
  const paragraphs = text.split(/\n\s*\n/);
  if (paragraphs.length > 1) {
    const asks = (p: string) => /\?\s*[*_)"'`]*$/.test(p.trim());
    const last = paragraphs.length - 1;
    const listy = /^\s*(?:[-*+]|\d+[.)])\s/.test(paragraphs[last]!);
    const from = asks(paragraphs[last]!) ? last : listy && asks(paragraphs[last - 1]!) ? last - 1 : -1;
    if (from >= 0) return parts(paragraphs.slice(from).join('\n\n').trim(), paragraphs.slice(0, from).join('\n\n').trim());
  }

  // 3. A short reason is the question. A long one: its first line is, and the rest is the report.
  if (text.length <= SHORT && lines.length === 1) return { ask: text, reason: '', report: '' };
  const head = lines.findIndex((l) => plain(l));
  return parts(plain(lines[head]!), join(lines.slice(head + 1)));
}

/** Short reports read inline; a long one waits behind "Read full report". */
export const isLongReport = (report: string) => report.length > 360 || report.split('\n').length > 6;
