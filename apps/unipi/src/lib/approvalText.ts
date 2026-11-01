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
