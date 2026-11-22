/**
 * "Open URL in the browser tab" (spec Z-106): the one entry point for
 * opening a web page inside the app — a new side-pane Browser tab on the
 * PC's own agent browser, beside the chat. Used by the terminal's web links.
 *
 * Falls back to the system browser (`openLink`) where there is no side pane
 * (phone shell, no workspace shown) or the browser extension is off.
 */
import { layout, desktopShell } from '../../lib/layout';
import { panes } from '../../lib/panes';
import { paneKind } from '../../lib/panes/kinds';
import { openLink } from '../../lib/markdown/links';
import { reservedGate } from '../../ui/slots/reserved';
import { currentWorkspace } from '../shell/Workspace';

/** Only http(s) pages go to the in-app browser. Pure. */
export function browsableUrl(url: string): boolean {
  return /^https?:\/\/\S+$/i.test(url.trim());
}

export type OpenUrlResult = 'tab' | 'external' | 'failed';

/** Opens `url` in a new Browser tab of the shown workspace (on `hostId`'s browser); else the system browser. */
export async function openUrlInBrowserTab(url: string, opts: { hostId: string }): Promise<OpenUrlResult> {
  const ws = currentWorkspace();
  const kind = paneKind('browser');
  const inApp = browsableUrl(url) && desktopShell() && !!ws && !!kind && (!kind.reserved || reservedGate(kind.reserved));
  if (inApp) {
    panes().open(ws!.scope, 'side', ws!.view, { kind: 'browser', params: { hostId: opts.hostId, url: url.trim() } });
    layout().toggleSide(true);
    return 'tab';
  }
  const r = await openLink(url);
  return r === 'failed' ? 'failed' : 'external';
}
