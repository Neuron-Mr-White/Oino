/**
 * Browser → chat context (browser.md §9.3–§9.4, WP-B8): what a picked element or a
 * screenshot becomes in the composer.
 *
 * ZCode's boundary: the chip is the visual shell, the prompt text is the context — a
 * picked element turns into one composer chip whose Markdown block (page, element,
 * selector, XPath, text, safe attributes, HTML excerpt) is flattened into the message on
 * send, plus the element's crop as a normal image attachment. A screenshot is just an
 * image attachment. Chips and images live in the chat's draft (per chat, in memory),
 * the same stores the composer's paste / drop use — nothing here touches the composer.
 */
import type { BrowserElementDetail } from '@unipi/protocol';
import { addReadyImage } from '../../lib/composer/attachments';
import { addComposerChip, focusComposer } from '../../lib/composer/drafts';
import type { BrowserPick, BrowserShot } from '../../lib/browserPane';

/** The composer key of a chat (`host/session`), or the host's new-chat draft. */
export function composerKeyOf(hostId: string, sessionId: string | undefined): string {
  return sessionId ? `${hostId}/${sessionId}` : `draft:${hostId}`;
}

const clip = (text: string, max: number): string => (text.length > max ? `${text.slice(0, max - 1)}…` : text);
/** Inline code that survives backticks in the value. */
const code = (text: string): string => {
  const fence = text.includes('`') ? '``' : '`';
  return `${fence}${text.includes('`') ? ` ${text} ` : text}${fence}`;
};

/** The chip label: `<button> Send` / `<h1> Hello from the probe`. */
export function elementLabel(pick: Pick<BrowserPick, 'element'>): string {
  const detail = pick.element.detail;
  const tag = detail?.tag ?? pick.element.label.split(/[#.]/)[0] ?? 'element';
  const name = (detail?.name ?? detail?.text ?? '').trim();
  return clip(name ? `<${tag}> ${name}` : `<${tag}> ${pick.element.label}`, 48);
}

/** The Markdown the model receives for a picked element. */
export function elementMarkdown(pick: BrowserPick): string {
  const d: Partial<BrowserElementDetail> = pick.element.detail ?? {};
  const url = d.url || pick.url;
  const title = (d.title || pick.title || '').trim();
  const lines: string[] = [];
  lines.push(`**Web element** picked in the browser${title || url ? ` on ${title ? `“${clip(title, 120)}”` : ''}${url ? ` (${url})` : ''}` : ''}`);
  const what = [code(pick.element.label)];
  if (d.role) what.push(`role ${code(d.role)}`);
  if (d.name) what.push(`name “${clip(d.name, 160)}”`);
  lines.push(`- Element: ${what.join(', ')}`);
  if (d.selector) lines.push(`- Selector: ${code(d.selector)}`);
  if (d.xpath) lines.push(`- XPath: ${code(d.xpath)}`);
  if (d.text && d.text !== d.name) lines.push(`- Text: “${clip(d.text, 400)}”`);
  const attrs = Object.entries(d.attributes ?? {}).filter(([k]) => k !== 'class' || !pick.element.label.includes('.'));
  if (attrs.length) lines.push(`- Attributes: ${attrs.map(([k, v]) => code(`${k}="${clip(v, 120)}"`)).join(' ')}`);
  lines.push(`- Box: ${Math.round(pick.element.width)}×${Math.round(pick.element.height)} px at (${Math.round(pick.element.x)}, ${Math.round(pick.element.y)})`);
  let out = lines.join('\n');
  if (d.html) out += `\n\n\`\`\`html\n${d.html.replace(/```/g, '`\u200b``')}\n\`\`\``;
  return out;
}

/** A picked element → a chip (+ its crop as an image) in the chat's composer. */
export function sendPickToComposer(key: string, pick: BrowserPick): void {
  addComposerChip(key, {
    kind: 'web-element',
    label: elementLabel(pick),
    detail: [pick.element.detail?.selector, pick.element.detail?.url ?? pick.url].filter(Boolean).join('\n'),
    markdown: elementMarkdown(pick),
  });
  if (pick.shot) addReadyImage(key, { name: `element ${pick.element.label}.png`.slice(0, 60), mime: pick.shot.mime, data: pick.shot.data });
  focusComposer(key);
}

/** A screenshot of the page → an image attachment in the chat's composer. */
export function sendShotToComposer(key: string, shot: BrowserShot, title: string): void {
  const name = `${(title || 'page').replace(/[\\/:*?"<>|]+/g, ' ').trim().slice(0, 40) || 'page'} screenshot.${shot.mime === 'image/png' ? 'png' : 'jpg'}`;
  addReadyImage(key, { name, mime: shot.mime, data: shot.data });
  focusComposer(key);
}
