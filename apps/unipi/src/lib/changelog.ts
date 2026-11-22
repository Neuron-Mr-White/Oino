/**
 * The app's own release notes (`apps/unipi/CHANGELOG.md`, bundled at build time): About ›
 * What's new and "Browse changelog" read them, so neither needs the update feed nor the
 * Updater extension (owner 2026-11-06).
 */
import raw from '../../CHANGELOG.md?raw';

export type ChangelogEntry = { version: string; date?: string; notes: string };

/** `## <version> — <date>` sections, newest first. Pure. */
export function parseChangelog(text: string): ChangelogEntry[] {
  const out: ChangelogEntry[] = [];
  let cur: { version: string; date?: string; lines: string[] } | undefined;
  for (const line of text.split('\n')) {
    const head = /^##\s+(\S+)(?:\s+[—–-]\s+(.+))?\s*$/.exec(line);
    if (head) {
      if (cur) out.push({ version: cur.version, ...(cur.date ? { date: cur.date } : {}), notes: cur.lines.join('\n').trim() });
      cur = { version: head[1]!, ...(head[2] ? { date: head[2].trim() } : {}), lines: [] };
    } else if (cur) {
      cur.lines.push(line);
    }
  }
  if (cur) out.push({ version: cur.version, ...(cur.date ? { date: cur.date } : {}), notes: cur.lines.join('\n').trim() });
  return out;
}

export const CHANGELOG_TEXT: string = raw;
export const changelog = (): ChangelogEntry[] => parseChangelog(CHANGELOG_TEXT);

/** The notes of `version` (this build), else the newest entry. */
export function notesFor(version: string | undefined): ChangelogEntry | undefined {
  const all = changelog();
  return all.find((e) => e.version === version) ?? all[0];
}
