/**
 * `browser` pane tab (browser.md §9, WP-B8): the PC's browser — the one the agent
 * drives with its `browser_*` tools — live in the side pane.
 *
 * The view is the proto screencast (`browser_open` stream: JPEG frames + acks, CDP
 * input relay), never a webview of the page (H11). On top of the imported viewer
 * (`screens/BrowserView`, variant `tab`): one toolbar row (back / forward / reload,
 * address, element picker, screenshot, profile, engine badge, ⋯), the PC's tab strip,
 * the "Agent is using the browser · Take over" strip, and the canvas. The tab follows
 * the agent: a tab the agent opens is shown, and the strip title is the page's title.
 *
 * Picker / screenshot land in this chat's composer (`elementContext`): a chip whose
 * Markdown the model receives on send, plus the crop as an image attachment.
 *
 * Registration (one line each, append-only): `lib/panes/kinds.ts` plugs this view in
 * lazily; `components/shell/PanesMenu.tsx` routes the phone's Panes → Browser to
 * `phoneLayer` (the full-height sheet).
 */
import { Show } from 'solid-js';
import { useHostsOptional } from '../../hosts/context';
import type { PaneTabProps } from '../../lib/panes/kinds';
import { browserPane } from '../../lib/browserPane';
import { displayUrl } from '../../lib/browser';
import { toasts } from '../../lib/toasts';
import { sessions } from '../../lib/session/store';
import { BrowserViewer } from '../../screens/BrowserView';
import { CommandBoxInline } from '../CommandBoxInline';
import { composerKeyOf, sendPickToComposer, sendShotToComposer } from './elementContext';
import './browser-tab.css';

/** The strip title of a browser tab for a page (`undefined` = the kind's "Browser"). */
export function browserTabTitle(page: { url: string; title: string }): string | undefined {
  const title = page.title.trim();
  if (title && title !== page.url && title !== 'about:blank') return title;
  return displayUrl(page.url) || undefined;
}

const TAB_URLS = 'unipi.browserTab.urls';
type TabUrls = Array<[tabId: string, url: string]>;

function readTabUrls(): TabUrls {
  try {
    const raw = JSON.parse(localStorage.getItem(TAB_URLS) ?? '[]') as unknown;
    return Array.isArray(raw) ? raw.filter((r): r is [string, string] => Array.isArray(r) && typeof r[0] === 'string' && typeof r[1] === 'string') : [];
  } catch {
    return [];
  }
}

/** Remembers the page a Browser pane tab shows, so it reopens there after a restart (newest 50 tabs). */
export function rememberTabUrl(tabId: string, url: string): void {
  if (!/^https?:/i.test(url)) return;
  const next = [...readTabUrls().filter(([id]) => id !== tabId), [tabId, url] as [string, string]].slice(-50);
  try {
    localStorage.setItem(TAB_URLS, JSON.stringify(next));
  } catch {
    // storage blocked: not remembered
  }
}

/** The page a Browser pane tab opens on: its last page, else the link it was opened for (`params.url`). */
export function tabStartUrl(tabId: string, paramUrl: unknown): string | undefined {
  const saved = readTabUrls().find(([id]) => id === tabId)?.[1];
  return saved ?? (typeof paramUrl === 'string' && paramUrl ? paramUrl : undefined);
}

export function BrowserTab(props: PaneTabProps) {
  const hosts = useHostsOptional();
  const pane = browserPane();
  const hostId = () => String(props.tab.params?.hostId ?? props.hostId);
  const hostName = () => hosts?.hosts().find((h) => h.id === hostId())?.name ?? 'this PC';
  const key = () => composerKeyOf(hostId(), props.sessionId);
  // Read once per mount: the viewer reads `url` several times while it opens.
  const startUrl = tabStartUrl(props.tab.id, props.tab.params?.url);
  /** E-15: any browser_* tool of this chat still running = the agent holds the tab. */
  const agentBusy = () => {
    const live = (props.sessionId ? sessions().peek(hostId(), props.sessionId) : undefined)?.state.liveTools ?? {};
    return Object.values(live).some((t) => !t.done && /^browser_/.test(t.name));
  };
  /** The shown page's title (names a screenshot). */
  let pageTitle = '';
  return (
    <div class="browser-tab-pane" data-testid="browser-tab">
      <Show when={`${hostId()}\u0000${pane.profileFor(hostId())}`} keyed>
        {(_key: string) => (
          <BrowserViewer
            variant="tab"
            hostId={hostId()}
            hostName={hostName()}
            url={startUrl}
            profile={pane.profileFor(hostId())}
            onProfile={(name) => pane.setProfileFor(hostId(), name)}
            CommandBox={CommandBoxInline}
            agentBusy={agentBusy}
            onClose={() => props.close()}
            onPage={(page) => {
              pageTitle = page.title;
              rememberTabUrl(props.tab.id, page.url);
              const title = browserTabTitle(page);
              if (title !== props.tab.title) props.setTitle(title);
            }}
            onPick={(pick) => {
              sendPickToComposer(key(), pick);
              toasts().push(`Added ${pick.element.label} to the chat`, 'info');
            }}
            onCapture={(shot) => {
              sendShotToComposer(key(), shot, pageTitle);
              toasts().push('Screenshot added to the chat', 'info');
            }}
          />
        )}
      </Show>
    </div>
  );
}

/** `lib/panes/kinds.ts` plugs this in lazily (`setPaneComponent('browser', lazy(…))`). */
export default BrowserTab;
