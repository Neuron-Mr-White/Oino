/**
 * Markdown link handling (UNI-170), shared by every markdown surface
 * (chat MdBlock, board Markdown, the previewer): find the anchor that
 * was tapped, decide whether it's external, and open/copy it. No UI
 * here — the LinkAnchorMenu component owns the long-press menu.
 */
import { copyText, openExternal } from '../board/share';

/** The anchor a pointer event hit, if it is inside the container. */
export function anchorFromEvent(event: { target: EventTarget | null }, container: HTMLElement): HTMLAnchorElement | null {
  const target = event.target as HTMLElement | null;
  const anchor = (target?.closest?.('a[href]') ?? null) as HTMLAnchorElement | null;
  if (!anchor || !container.contains(anchor)) return null;
  return anchor;
}

/** http(s)/mailto links open outside the app; anything else is inert. */
export function isExternalHref(href: string): boolean {
  return /^(https?|mailto):/i.test(href);
}

export type OpenLinkResult = 'opened' | 'copied' | 'failed';

/** Opens a link externally; falls back to copying it. */
export async function openLink(href: string): Promise<OpenLinkResult> {
  if (openExternal(href)) return 'opened';
  return (await copyText(href)) ? 'copied' : 'failed';
}
