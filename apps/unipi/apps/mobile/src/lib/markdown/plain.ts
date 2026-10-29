/**
 * Markdown → one line of plain text (board card excerpts, UNI-130/156):
 * no `**`, backticks, list markers or table pipes.
 */
import { renderMarkdown } from './core';

export function plainText(text: string | undefined): string {
  const html = renderMarkdown(text);
  if (!html) return '';
  const doc = new DOMParser().parseFromString(html.replace(/<\/(p|li|h\d|tr)>|<br\s*\/?>/g, ' $&'), 'text/html');
  const raw = (doc.body.textContent ?? '').replace(/\s+/g, ' ').trim();
  // Markdown that reached here unrendered (plain `**x**`, `` `y` ``, table
  // pipes inside a code/diff block the regex above doesn't touch) is still
  // stripped so excerpts never show stray syntax.
  return raw.replace(/\*\*/g, '').replace(/`/g, '').replace(/\|/g, ' ').replace(/\s+/g, ' ').trim();
}
