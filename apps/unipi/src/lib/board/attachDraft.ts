/**
 * Attachments in a text field (D2 / P3-1): a pasted or picked file is a chip, like the chat composer's
 * attachment chips — the field shows the words only. The saved text still carries the markdown link
 * (`![shot.png](att:T-1/…)` or `[log.txt](att:…)`, `pending:<n>` until a not-yet-created / not-yet-saved task has stored it),
 * on lines of their own at the END of the body: `text` + `\n` + one link per line. `splitRefs` reads that
 * shape back, `joinRefs` writes it; a body without such a tail is all text (links in the middle of a
 * description stay as the author wrote them).
 */
export type DraftRef = {
  /** The markdown line, as stored. */
  markdown: string;
  image: boolean;
  /** The link text (the file's name). */
  name: string;
  /** `att:<TASK>/<stored name>` or `pending:<id>`. */
  target: string;
};

const REF_LINE = /^[ \t]*(!?)\[([^\]]*)\]\(((?:att|pending):[^)\s]*)\)[ \t]*$/;

export function parseRef(line: string): DraftRef | undefined {
  const m = REF_LINE.exec(line);
  return m ? { markdown: line.trim(), image: m[1] === '!', name: m[2]!, target: m[3]! } : undefined;
}

/** The body's words and the attachment links that close it. */
export function splitRefs(body: string): { text: string; refs: DraftRef[] } {
  const lines = body.split('\n');
  let at = lines.length;
  while (at > 0 && REF_LINE.test(lines[at - 1]!)) at--;
  if (at === lines.length) return { text: body, refs: [] };
  return { text: lines.slice(0, at).join('\n'), refs: lines.slice(at).map((line) => parseRef(line)!) };
}

export function joinRefs(text: string, refs: readonly DraftRef[]): string {
  if (refs.length === 0) return text;
  const tail = refs.map((ref) => ref.markdown).join('\n');
  return text === '' ? tail : `${text}\n${tail}`;
}

/** `att:T-1/ab12cd34-shot.png` → `{ task: 'T-1', name: 'ab12cd34-shot.png' }`. */
export function attTarget(target: string): { task: string; name: string } | undefined {
  const m = /^att:([^/]+)\/(.+)$/.exec(target);
  return m ? { task: m[1]!, name: m[2]! } : undefined;
}

export const pendingId = (ref: DraftRef): string | undefined => (ref.target.startsWith('pending:') ? ref.target.slice('pending:'.length) : undefined);
