/**
 * E-07: an approval card shows what the agent wants to do in words, never the
 * tool's raw JSON arguments. Extension text such as
 * `{"name":"Write marker","command":"echo hi > a.txt","isAgent":false}` becomes
 * `Run: echo hi > a.txt`; paths become `Edit <path>`.
 */
const asObject = (text: string): Record<string, unknown> | undefined => {
  const t = text.trim();
  if (!t.startsWith('{') || !t.endsWith('}')) return undefined;
  try {
    const v: unknown = JSON.parse(t);
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : undefined;
  } catch {
    return undefined;
  }
};

const s = (v: unknown): string | undefined => (typeof v === 'string' && v.trim() ? v.trim() : undefined);
const clip = (t: string, n: number): string => {
  const flat = t.replace(/\s+/g, ' ');
  return flat.length > n ? `${flat.slice(0, n - 1)}…` : flat;
};

/** A one-line human summary of tool args (JSON text) — other text passes through unchanged. */
export function humanizeArgs(text: string, max = 120): string {
  const o = asObject(text);
  if (!o) {
    // Truncated JSON (the host clips long args): still never show the braces.
    const m = /^\s*\{\s*"(?:name|command)"\s*:/.test(text) ? /"command"\s*:\s*"((?:[^"\\]|\\.)*)/.exec(text) : null;
    return m ? `Run: ${clip(m[1]!.replace(/\\n/g, ' ').replace(/\\"/g, '"'), max)}` : text;
  }
  const cmd = s(o.command) ?? s(o.cmd);
  if (cmd) return `Run: ${clip(cmd, max)}`;
  const path = s(o.path) ?? s(o.file_path) ?? s(o.filePath) ?? s(o.file);
  if (path) return `${o.content !== undefined || o.oldText !== undefined || o.edits !== undefined ? 'Edit' : 'Use'} ${clip(path, max)}`;
  const url = s(o.url);
  if (url) return `Open ${clip(url, max)}`;
  const name = s(o.name) ?? s(o.title) ?? s(o.description);
  if (name) return clip(name, max);
  const pairs = Object.entries(o)
    .filter(([, v]) => typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean')
    .slice(0, 3)
    .map(([k, v]) => `${k}: ${String(v)}`);
  return pairs.length ? clip(pairs.join(' · '), max) : 'Use this tool';
}

/** Button label: backticked JSON in "Always allow `{…}`" is replaced by its readable form. */
export function humanizeLabel(label: string): string {
  return label.replace(/`([^`]*)`/g, (_m, inner: string) => {
    const h = humanizeArgs(inner, 40);
    return h === inner ? `\`${inner}\`` : `“${h.replace(/^Run: /, '')}”`;
  });
}

/** The full command/path/diff to show in the detail area when the card has none. */
export function detailOf(text: string): { kind: 'command' | 'path'; text: string } | undefined {
  const o = asObject(text);
  if (!o) return undefined;
  const cmd = s(o.command) ?? s(o.cmd);
  if (cmd) return { kind: 'command', text: cmd };
  const path = s(o.path) ?? s(o.file_path) ?? s(o.filePath);
  return path ? { kind: 'path', text: path } : undefined;
}

type AskQuestion = { question?: unknown; header?: unknown; options?: unknown };

/** The fallback `summary` the bash tool writes for a call that sent none: not a title worth showing (N8). */
const NO_SUMMARY = new Set(['(legacy call)', 'legacy call']);

/**
 * What an approval card shows for a call: a title in words and the detail block (command / questions / JSON).
 *  - N8: a bash call without a (real) summary is titled by its command, never "(legacy call)".
 *  - N10: an `ask_user` approval renders the questions and options, not the raw JSON arguments.
 */
/** The first lines of a file body, with how much more there is (G3-7: a write approval previews the file, never its raw JSON). */
function preview(text: string, maxLines = 14, maxChars = 1_200): string {
  const lines = text.split('\n');
  let shown = lines.slice(0, maxLines).join('\n');
  if (shown.length > maxChars) shown = shown.slice(0, maxChars);
  const cutLines = lines.length - shown.split('\n').length;
  const cutChars = text.length - shown.length;
  if (cutChars <= 0) return shown;
  return `${shown}\n… ${cutLines > 0 ? `${cutLines.toLocaleString('en-US')} more line${cutLines === 1 ? '' : 's'} · ` : ''}${text.length.toLocaleString('en-US')} characters in all`;
}

const fileOf = (args: Record<string, unknown>): string | undefined => s(args.path) ?? s(args.file_path) ?? s(args.filePath);

export function approvalView(tool: string, args: Record<string, unknown>): { title?: string; detail: string; kind: 'command' | 'questions' | 'preview' | 'json' } {
  const summary = s(args.summary);
  const real = summary && !NO_SUMMARY.has(summary.toLowerCase()) ? summary : undefined;
  const qs = Array.isArray(args.questions) ? (args.questions as AskQuestion[]).filter((q) => q && typeof q === 'object' && s(q.question)) : [];
  if (qs.length) {
    const lines = qs.flatMap((q, i) => {
      const opts = Array.isArray(q.options) ? q.options.map((o) => (o && typeof o === 'object' ? s((o as { label?: unknown }).label) : s(o))).filter((o): o is string => !!o) : [];
      return [`${qs.length > 1 ? `${i + 1}. ` : ''}${s(q.question)!}`, ...(opts.length ? [`   ${opts.join(' · ')}`] : [])];
    });
    return { title: real ?? (qs.length === 1 ? `Question: ${clip(s(qs[0]!.question)!, 100)}` : `${qs.length} questions`), detail: lines.join('\n'), kind: 'questions' };
  }
  // G3-7: a write / edit shows what it does to which file, with a preview of the content.
  const file = fileOf(args);
  if (file && typeof args.content === 'string') return { title: real ?? `Write ${clip(file, 100)}`, detail: preview(args.content), kind: 'preview' };
  if (file && Array.isArray(args.edits) && args.edits.length) {
    const edits = args.edits as Array<{ oldText?: unknown; newText?: unknown }>;
    const body = edits.slice(0, 3).map((e, i) => `${edits.length > 1 ? `${i + 1}. ` : ''}${preview(typeof e?.newText === 'string' ? e.newText : '', 6, 500)}`).join('\n\n');
    return { title: real ?? `Edit ${clip(file, 100)}${edits.length > 1 ? ` (${edits.length} changes)` : ''}`, detail: edits.length > 3 ? `${body}\n\n… ${edits.length - 3} more changes` : body, kind: 'preview' };
  }
  const command = s(args.command);
  if (command) return { title: real ?? `Run: ${clip(command, 100)}`, detail: command, kind: 'command' };
  return { title: real, detail: JSON.stringify(args, null, 2), kind: 'json' };
}
