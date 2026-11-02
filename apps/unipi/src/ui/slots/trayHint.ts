/**
 * E-17: the closed-tray hint ("Background tasks 4 · Subagents 2 · ↓ open") counts every record
 * a tab still holds, so it kept saying "Subagents 2" long after the work ended. The app shows
 * only what is running now — "Subagents 1 running · ↓ open" — and hides the line when nothing is.
 */
import type { UiEntry } from '../types';

const OPEN = '↓ open';
const PART = /^(.*?)\s+(\d+)(?:\s+running|\s+\((\d+)\s+running\)|\/(\d+))?$/;

/** The live version of one hint line; `null` = nothing running (hide it); `undefined` = not a tray hint. */
export function liveTrayLine(line: string): string | null | undefined {
  const text = line.trim();
  if (!text.endsWith(OPEN)) return undefined;
  const parts = text.slice(0, -OPEN.length).split(' · ').map((p) => p.trim()).filter(Boolean);
  if (!parts.length) return undefined;
  const live: string[] = [];
  for (const part of parts) {
    const m = PART.exec(part);
    if (!m) return undefined;
    const [, label, total, parenRunning, slashRunning] = m;
    const running = /\brunning$/.test(part) && !parenRunning ? Number(total) : Number(parenRunning ?? slashRunning ?? 0);
    if (running > 0) live.push(`${label} ${running} running`);
  }
  return live.length ? `${live.join(' · ')} · ${OPEN}` : null;
}

/** Widget entries with stale tray hints rewritten / dropped. */
export function liveTrayEntries(entries: UiEntry[]): UiEntry[] {
  const out: UiEntry[] = [];
  for (const e of entries) {
    if (e.view.kind !== 'lines') {
      out.push(e);
      continue;
    }
    const lines = e.view.lines;
    const live = lines.map(liveTrayLine);
    if (!live.some((l) => l !== undefined)) {
      out.push(e);
      continue;
    }
    const kept = lines.flatMap((l, i) => (live[i] === undefined ? [l] : live[i] === null ? [] : [live[i]!]));
    if (kept.length) out.push({ ...e, view: { ...e.view, lines: kept } } as UiEntry);
  }
  return out;
}
