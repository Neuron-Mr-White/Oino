/**
 * Markdown → sanitised HTML for task bodies, notes and reports (UNI-130,
 * UNI-156). Thin board-specific glue over the shared renderer
 * (lib/markdown): `att:<TASK>/<name>` refs resolve against the open
 * project and the phone can't load `/api/files/…` directly (the daemon is
 * behind the board transport), so rendered media carry a `data-att-path`
 * attribute and no live `src` — call `hydrateAttachments(container,
 * hostId)` after inserting the HTML to swap in object URLs.
 */
import { ATT_REF, attDisplayName, attKind, hydrateAttachments as hydrateAttachmentsShared, plainText as plainTextShared, renderMarkdown as renderMarkdownShared } from '../markdown';
import { api, filePath } from './api';

export { attDisplayName, attKind, type AttKind } from '../markdown';

let projectSlug = '';
/** Set when a project opens: att: refs resolve against it. */
export function setAttachmentBase(slug: string | null): void {
  projectSlug = slug ?? '';
}

/** `att:UNI-4/abc-shot.png` → `/api/files/<slug>/UNI-4/abc-shot.png` (daemon path), or null. */
export function attPath(reference: string): string | null {
  const match = ATT_REF.exec(reference.trim());
  if (!match || !projectSlug) return null;
  return filePath(projectSlug, match[1]!, match[2]!);
}

export function renderMarkdown(text: string | undefined): string {
  return renderMarkdownShared(text, { resolveAttachment: attPath, allowImages: true });
}

/** Markdown → one line of plain text (card callouts): no `**`, backticks, list markers or table pipes. */
export function plainText(text: string | undefined): string {
  return plainTextShared(text);
}

/**
 * Resolves `data-att-path` media inside `root` to object URLs fetched
 * through the board transport (images/video/audio get a src; chips and
 * image links get an href). Safe to call repeatedly.
 */
export async function hydrateAttachments(root: HTMLElement, hostId: string): Promise<void> {
  await hydrateAttachmentsShared(root, (path) => api.fileUrl(hostId, path));
}

/** "3m ago", "2h ago", "5d ago" — short, tabular-friendly. */
export function relativeTime(iso: string | undefined, now = Date.now()): string {
  if (!iso) return '';
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return '';
  const seconds = Math.max(0, Math.round((now - then) / 1000));
  if (seconds < 45) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}
