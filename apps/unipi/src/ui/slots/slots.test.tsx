// @vitest-environment jsdom
/**
 * WP-U10 slot hosts (ui-protocol §6.2 acceptance: per-slot
 * open / update / close / dispose / order / two contributors; G-1 and G-4 of
 * zcode-audit): driven through the real registry (`oino_ui_patch` events on a
 * mock session stream), asserting each host surface and the commands the
 * hosts emit (`$close`, `_oino/ui_visible`, palette "Open <title>").
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createMemo, createSignal, Show } from 'solid-js';
import { Route, Router } from '@solidjs/router';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createMockHost, type MockWorld } from '../../dev/mock/host';
import { resetSessions, sessions, type SessionHandle } from '../../lib/session/store';
import { setHostClientFactory } from '../../lib/host/factory';
import type { SessionSnapshot } from '../../lib/host/client';
import { panes, resetPanes } from '../../lib/panes';
import { paneKind } from '../../lib/panes/kinds';
import { allCommands } from '../../lib/shell';
import { railItems } from '../../components/shell/Rail';
import { setDesktopOverride } from '../../lib/layout';
import { toasts } from '../../lib/toasts';
import { composerDraft, resetComposerDrafts } from '../../lib/composer/drafts';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';
import { resetLocal } from '../localState';
import { installUiSinks, resetUiRegistries, sessionUi } from '../registries';
import { setUiTransport, type UiTransport } from '../actions';
import { WidgetStack } from './WidgetStack';
import { SlotModal, resetOverlayDismissals } from './SlotModal';
import { ComposerTray, resetTrayDismissals } from './ComposerTray';
import { ExtPaneTab, PhonePaneLayer, PhonePanesGroup, reconcilePaneTabs, resetPaneClosures, wireExtPaneKind } from './paneTabs';
import { ExtensionPage, feedExtensionPages, installExtensionPageFeed } from './pages';
import { feedPaletteEntries } from './paletteFeed';
import { feedStatusStrip } from './statusFeed';
import { TurnFooters } from './TurnFooters';
import { installUiEffects, resetUiEffects } from './effects';
import { readyImages } from '../../lib/composer/attachments';
import { subscribeEditorTokens, resetEditorTokens } from '../../lib/composer/tokens';
import { consumePendingSettingsNamespace } from '../../lib/settings/open';
import { setPlatformOverride } from '../../lib/platform';
import { clearStatusScope, statusSegments, StatusStrip } from '../../components/shell/StatusStrip';
import { BottomDock } from '../../components/dock/BottomDock';
import { resetSessionRenderCaches } from '../../lib/session/renderCache';
import type { TimelineCtx } from '../../components/timeline/context';
import type { Turn } from '../../lib/session/turns';
import type { EntryView, UiEntry } from '../types';

const HOST = 'h1';
const SCOPE = `${HOST}/s1`;
const TARGET = { scope: 'session' as const, hostId: HOST, sessionId: 's1' };

const rt = roundtrip as unknown as { fresh: SessionSnapshot };
/** The fixture snapshot without its `ui` block: the registry starts at rev 0, entries empty. */
const baseSnapshot = (): SessionSnapshot => ({ ...rt.fresh, seq: 0, entries: [], ui: undefined });

let world_: MockWorld;
let session: SessionHandle | undefined;
let disposers: Array<() => void> = [];
let sent: Array<{ entryId: string; actionId: string; visible?: boolean }> = [];

beforeEach(() => {
  localStorage.clear();
  resetUiRegistries();
  resetUiEffects();
  resetLocal();
  resetPanes();
  resetSessionRenderCaches();
  resetComposerDrafts();
  resetEditorTokens();
  resetOverlayDismissals();
  resetTrayDismissals();
  resetPaneClosures();
  clearStatusScope(SCOPE);
  toasts().clear();
  sent = [];
  installUiSinks();
  installUiEffects();
  setUiTransport({
    action: (_t, p) => {
      sent.push({ entryId: p.entryId, actionId: p.actionId });
      return Promise.resolve({});
    },
    visible: (_t, entryId, visible) => {
      sent.push({ entryId, actionId: '$visible', visible });
      return Promise.resolve({});
    },
    resync: () => Promise.resolve({}),
  } satisfies UiTransport);
  setDesktopOverride(true);
  world_ = createMockHost({
    name: 'slots',
    hosts: [
      {
        id: HOST,
        name: 'coffee',
        renderers: { 'turnFooter/files': { t: 'Paragraph', text: 'edited src/ui/slots/WidgetStack.tsx · +84 −12' } },
        sessions: { s1: { snapshot: baseSnapshot(), events: [] } },
      },
    ],
  });
  setHostClientFactory(world_.client);
});

afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  session = undefined;
  cleanup();
  resetSessions();
  setHostClientFactory(undefined);
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
});

/** Opens the mock session through the app's session registry (as Conversation does). */
async function open(): Promise<SessionHandle> {
  const s = sessions().ensure(HOST, 's1');
  session = s;
  await waitFor(() => expect(s.state.conn).toBe('live'));
  return s;
}

// ── Registry wire helpers (seq/rev continue from the session's snapshot) ──

let seq = 0;
let rev = 0;
let entrySeq = 0;

const pushOps = (ops: unknown[]) =>
  world_.push(HOST, 's1', [{ session: 's1', seq: ++seq, event: { type: 'oino_ui_patch', scope: 'session', rev: ++rev, ops } }]);

const put = (entry: Partial<UiEntry> & Pick<UiEntry, 'id' | 'slot' | 'view'>) =>
  pushOps([{ op: 'put', entry: { extensionId: 'ext', scope: 'session', order: 0, seq: ++entrySeq, rev: 1, ...entry } }]);
const del = (id: string) => pushOps([{ op: 'del', id }]);
const meta = (id: string, set: Record<string, unknown>) => pushOps([{ op: 'meta', id, set }]);
const effect = (request: Record<string, unknown>) => pushOps([{ op: 'effect', request: { id: `e${++entrySeq}`, ...request } }]);

const tree = (text: string): EntryView => ({ kind: 'tree', root: { t: 'Paragraph', text } });

/** Opens the session and keeps the wire's seq/rev in step with the registry. */
async function openTracking() {
  const s = await open();
  seq = 0;
  rev = sessionUi(HOST, 's1').state.rev;
  entrySeq = 0;
  return s;
}

// ── effects (§3.2): notify → the one toast surface ───────────────────────

describe('U10 one-shot effects', () => {
  it('notify lands in the app toast stack with its tone', async () => {
    await openTracking();
    effect({ method: 'notify', message: 'compactor: context trimmed 12 %', notifyType: 'warning' });
    await waitFor(() => expect(toasts().list().map((t) => `${t.kind}:${t.message}`)).toContain('warning:compactor: context trimmed 12 %'));
  });

  it('set_editor_text lands in the composer draft (undoable there)', async () => {
    const s = await openTracking();
    effect({ method: 'set_editor_text', text: 'from the extension' });
    await waitFor(() => expect(composerDraft(s.key).text).toBe('from the extension'));
  });

  it('openSettings routes dedicated capability namespaces and extension namespaces', async () => {
    await openTracking();
    effect({ method: 'openSettings', namespace: 'skills' });
    await waitFor(() => expect(location.hash).toBe('#/settings/skills'));

    effect({ method: 'open_settings', namespace: 'updater' });
    await waitFor(() => expect(location.hash).toBe('#/settings/extensions'));
    expect(consumePendingSettingsNamespace()).toBe('updater');
  });

  it('openUrl / open_url opens external http(s) links', async () => {
    await openTracking();
    const openSpy = Object.assign(window, { open: (() => ({} as Window)) as typeof window.open });
    let openedUrl = '';
    openSpy.open = ((url?: string | URL) => {
      openedUrl = String(url ?? '');
      return {} as Window;
    }) as typeof window.open;
    effect({ method: 'openUrl', url: 'https://github.com/oino-dev/oino/releases/tag/v2.0.0' });
    await waitFor(() => expect(openedUrl).toBe('https://github.com/oino-dev/oino/releases/tag/v2.0.0'));

    effect({ method: 'open_url', url: 'http://127.0.0.1:41234/a/token123' });
    await waitFor(() => expect(openedUrl).toBe('http://127.0.0.1:41234/a/token123'));
  });

  it('clipboardWrite writes text to the clipboard', async () => {
    await openTracking();
    let copied = '';
    Object.assign(navigator, {
      clipboard: {
        writeText: async (t: string) => {
          copied = t;
        },
      },
    });
    effect({ method: 'clipboardWrite', text: 'copied-from-effect' });
    await waitFor(() => expect(copied).toBe('copied-from-effect'));
  });

  it('osNotify dispatches to the native notification bridge', async () => {
    setPlatformOverride('tauri-mobile');
    const shown: Array<{ title: string; body: string }> = [];
    Object.defineProperty(window, 'UniPiNotify', {
      configurable: true,
      writable: true,
      value: {
        show: (_id: number, title: string, body: string) => {
          shown.push({ title, body });
        },
      },
    });
    await openTracking();
    effect({ method: 'osNotify', title: 'Agent graph work failed', body: 'Node wp-1 exited 1' });
    await waitFor(() => expect(shown).toEqual([{ title: 'Agent graph work failed', body: 'Node wp-1 exited 1' }]));
  });

  it('insertEditorToken updates the session draft/attachments and notifies composer subscribers', async () => {
    const s = await openTracking();
    effect({ method: 'insertEditorToken', token: { tokenId: 'tok1', kind: 'file', label: 'main.ts', path: 'src/main.ts' } });
    effect({
      method: 'insertEditorToken',
      token: { tokenId: 'img1', kind: 'image', label: 'cap.png', data: { mimeType: 'image/png', data: 'aGVsbG8=' } },
    });
    await waitFor(() => expect(composerDraft(s.key).text).toBe('@src/main.ts '));
    expect(readyImages(s.key)).toEqual([{ mimeType: 'image/png', data: 'aGVsbG8=' }]);

    const liveTokens: string[] = [];
    const off = subscribeEditorTokens(s.key, (tok) => liveTokens.push(tok.label));
    effect({ method: 'insertEditorToken', token: { kind: 'text', label: 'live-caret' } });
    await waitFor(() => expect(liveTokens).toEqual(['live-caret']));
    off();
  });
});

// ── status slot (G-4) + footer folded into the strip (rule 4.2.2) ────────

describe('U10 status strip (§6.2 status/footer rows)', () => {
  it('two contributors land in registry order; meta tone + delete apply live', async () => {
    await openTracking();
    render(() => {
      feedStatusStrip({ scope: () => SCOPE, registry: () => sessionUi(HOST, 's1') });
      return <StatusStrip scope={SCOPE} />;
    });
    put({ id: 'ext/status:a', slot: 'status', order: 10, view: { kind: 'status', text: 'main ↑1' } });
    put({ id: 'ext/status:b', slot: 'status', order: 20, view: { kind: 'status', text: '2 jobs' } });
    put({ id: 'ext/footer:k', slot: 'footer', title: 'ctx', view: tree('ctx 41 %') });
    await waitFor(() => expect(statusSegments(SCOPE).map((s) => s.text)).toEqual(['2 jobs', 'main ↑1', 'ctx']));
    // a footer entry renders its tree inside the one strip, one row
    expect(screen.getByTestId('status-strip')).toBeTruthy();
    // a `meta`-only change re-feeds the strip (no entry added or deleted)
    meta('ext/status:b', { tone: 'error' });
    await waitFor(() => expect(statusSegments(SCOPE).find((s) => s.key === 'ext/status:b')).toMatchObject({ tone: 'error' }));
    del('ext/status:a');
    await waitFor(() => expect(statusSegments(SCOPE).map((s) => s.text)).toEqual(['2 jobs', 'ctx']));
  });
});

// ── widget slot (two contributors, order/update/close, phone fold) ───────

describe('U10 widget slot', () => {
  const widgetsOf = () => sessionUi(HOST, 's1').entriesFor('widget')();

  it('renders contributors order desc then seq asc; tree update and delete apply', async () => {
    await openTracking();
    render(() => <WidgetStack entries={widgetsOf()} target={TARGET} />);
    put({ id: 'ext/widget:lo', slot: 'widget', order: 0, view: tree('fusion: 2 jobs') });
    put({ id: 'util/widget:hi', slot: 'widget', order: 10, view: tree('CPU 62 %') });
    await waitFor(() => expect(screen.getByText('CPU 62 %')).toBeTruthy());
    const stack = screen.getByTestId('ui-widgets');
    expect(stack.textContent!.indexOf('CPU')).toBeLessThan(stack.textContent!.indexOf('fusion'));
    pushOps([{ op: 'tree', id: 'util/widget:hi', rev: 2, ops: [{ op: 'set', path: [], props: { text: 'CPU 81 %' } }] }]);
    await waitFor(() => expect(screen.getByText('CPU 81 %')).toBeTruthy());
    del('util/widget:hi');
    await waitFor(() => expect(screen.queryByText('CPU 81 %')).toBeNull());
    expect(screen.getByText('fusion: 2 jobs')).toBeTruthy();
  });

  it('phone: beyond 3 entries the rest fold behind "+N widgets" (§6.4 cap)', async () => {
    setDesktopOverride(false);
    await openTracking();
    render(() => <WidgetStack entries={widgetsOf()} target={TARGET} phone />);
    [1, 2, 3, 4, 5].forEach((i) => put({ id: `ext/widget:${i}`, slot: 'widget', order: -i, view: tree(`w${i}`) }));
    await waitFor(() => expect(screen.getByText('w1')).toBeTruthy());
    expect(screen.queryByText('w4')).toBeNull();
    fireEvent.click(screen.getByTestId('ui-widgets-more'));
    await waitFor(() => expect(screen.getByText('w4')).toBeTruthy());
    expect(screen.getByText('w5')).toBeTruthy();
  });
});

// ── overlay slot: stack, visibility, ✕/Esc → $close, dispose closes ──────

describe('U10 overlay slot', () => {
  it('shows the stack top, reports visibility and $closes on Esc / ✕', async () => {
    await openTracking();
    const reg = sessionUi(HOST, 's1');
    render(() => (
      <Show when={reg.stack('overlay')()}>
        {(stack) => <SlotModal stack={stack()} target={TARGET} />}
      </Show>
    ));
    put({ id: 'ext/panel:a', slot: 'overlay', title: 'First', view: tree('first panel') });
    put({ id: 'ext/panel:b', slot: 'overlay', title: 'Second', view: tree('second panel') });
    await waitFor(() => expect(screen.getByTestId('ui-modal')).toBeTruthy());
    // the most recently opened entry is on top (§6.4)
    expect(screen.getByText('second panel')).toBeTruthy();
    expect(screen.queryByText('first panel')).toBeNull();
    await waitFor(() => expect(sent.some((s) => s.entryId === 'ext/panel:b' && s.actionId === '$visible' && s.visible === true)).toBe(true));
    fireEvent.keyDown(screen.getByTestId('ui-modal-scrim'), { key: 'Escape' });
    await waitFor(() => expect(sent).toContainEqual({ entryId: 'ext/panel:b', actionId: '$close' }));
    // core deletes it: the next entry of the stack takes over
    del('ext/panel:b');
    await waitFor(() => expect(screen.getByText('first panel')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Close First' }));
    await waitFor(() => expect(sent).toContainEqual({ entryId: 'ext/panel:a', actionId: '$close' }));
    // An owner that keeps the entry after `$close` (the updater's changelog) still leaves the screen.
    await waitFor(() => expect(screen.queryByTestId('ui-modal')).toBeNull());
  });

  it('unmounts cleanly inside a parent memo scope without throwing in Solid cleanNode', async () => {
    await openTracking();
    put({ id: 'ext/panel:unmount', slot: 'overlay', title: 'Unmount', view: tree('body') });
    const [mounted, setMounted] = createSignal(true);
    render(() => (
      <Show when={mounted()}>
        {(() => {
          const reg = createMemo(() => sessionUi(HOST, 's1'));
          const stack = () => reg().stack('overlay')();
          const tray = () => reg().stack('editorReplace')();
          return (
            <>
              <SlotModal stack={stack()} target={TARGET} />
              <ComposerTray stack={tray()} target={TARGET} />
            </>
          );
        })()}
      </Show>
    ));
    await waitFor(() => expect(screen.getByTestId('ui-modal')).toBeTruthy());
    expect(() => setMounted(false)).not.toThrow();
    await waitFor(() => expect(sent).toContainEqual({ entryId: 'ext/panel:unmount', actionId: '$visible', visible: false }));
  });

  it('phone: an overlay is a bottom sheet, `large` fills it (§13.6 row 6)', async () => {
    setDesktopOverride(false);
    await openTracking();
    const reg = sessionUi(HOST, 's1');
    render(() => (
      <Show when={reg.stack('overlay')()}>
        {(stack) => <SlotModal stack={stack()} target={TARGET} phone />}
      </Show>
    ));
    put({ id: 'ext/panel:big', slot: 'overlay', title: 'Big', size: 'large', view: tree('big panel') });
    await waitFor(() => expect(screen.getByTestId('ui-modal')).toBeTruthy());
    expect(screen.getByTestId('ui-modal').hasAttribute('data-full')).toBe(true);
  });
});

// ── editorReplace tray: replaces only the composer (app spec §3.11) ──────

describe('U10 editorReplace tray', () => {
  it("takes the composer's place in the dock, $closes on Esc and restores it", async () => {
    const s = await openTracking();
    const reg = sessionUi(HOST, 's1');
    const tray = () => reg.stack('editorReplace')();
    render(() => (
      <BottomDock
        session={s}
        composerTray={tray().length ? <ComposerTray stack={tray()} target={TARGET} /> : undefined}
        composer={<input data-testid="real-composer" aria-label="composer" />}
      />
    ));
    expect(screen.getByTestId('real-composer')).toBeTruthy();
    put({ id: 'core/panel:work', slot: 'editorReplace', title: 'Work tray', view: tree('background work') });
    await waitFor(() => expect(screen.getByTestId('ui-tray')).toBeTruthy());
    expect(screen.queryByTestId('real-composer')).toBeNull();
    expect(screen.getByText('background work')).toBeTruthy();
    fireEvent.keyDown(screen.getByTestId('ui-tray'), { key: 'Escape' });
    await waitFor(() => expect(sent).toContainEqual({ entryId: 'core/panel:work', actionId: '$close' }));
    // the entry going away restores the composer (§6.6)
    del('core/panel:work');
    await waitFor(() => expect(screen.getByTestId('real-composer')).toBeTruthy());
  });
});

// ── sidePane / bottomPane entries ↔ the Z-5 tabs container ──────────────

describe('U10 pane slots', () => {
  const scope = { hostId: HOST, projectKey: '~' };
  const view = { sessionId: 's1' };

  it('opens, retitles, badges and closes ext tabs with the registry', async () => {
    await openTracking();
    const reg = sessionUi(HOST, 's1');
    render(() => {
      reconcilePaneTabs({ hostId: HOST, sessionId: 's1', cwd: () => undefined, registry: () => reg });
      return <p>host</p>;
    });
    wireExtPaneKind();
    put({ id: 'ext/panel:board', slot: 'sidePane', title: 'Board panel', view: tree('board rows') });
    put({ id: 'ext/panel:notes', slot: 'sidePane', order: 50, title: 'Notes', badge: '3', focus: true, view: tree('notes body') });
    put({ id: 'ext/panel:build', slot: 'bottomPane', title: 'Build', view: tree('build log') });
    const store = panes();
    await waitFor(() =>
      expect(store.visible(scope, 'side', view).filter((t) => t.kind === 'ext').map((t) => t.title)).toEqual(expect.arrayContaining(['Notes', 'Board panel'])),
    );
    expect(store.visible(scope, 'bottom', view).some((t) => t.kind === 'ext' && t.params?.entryId === 'ext/panel:build')).toBe(true);
    // update: meta retitle + badge land on the tab
    meta('ext/panel:notes', { title: 'Scratch', badge: '5' });
    await waitFor(() => {
      const tab = store.visible(scope, 'side', view).find((t) => t.params?.entryId === 'ext/panel:notes');
      expect(tab).toMatchObject({ title: 'Scratch', badge: '5' });
    });
    // a title-only meta update retitles the tab (no other entry traffic)
    meta('ext/panel:board', { title: 'Kanban' });
    await waitFor(() => expect(store.visible(scope, 'side', view).find((t) => t.params?.entryId === 'ext/panel:board')?.title).toBe('Kanban'));
    // the strip's ✕ runs the kind's onUserClose → the entry gets `$close` (§6.2)
    const notes = store.visible(scope, 'side', view).find((t) => t.params?.entryId === 'ext/panel:notes')!;
    paneKind('ext')!.onUserClose!(notes);
    store.close(scope, 'side', view, notes.id);
    await waitFor(() => expect(sent).toContainEqual({ entryId: 'ext/panel:notes', actionId: '$close' }));
    // a user-closed tab is not re-opened while the entry lives on (§6.4 `id:seq`)
    meta('ext/panel:notes', { badge: '6' });
    await new Promise((r) => setTimeout(r, 20));
    expect(store.visible(scope, 'side', view).some((t) => t.params?.entryId === 'ext/panel:notes')).toBe(false);
    // …and a deleted entry closes its tab (§6.6 dispose)
    del('ext/panel:board');
    await waitFor(() => expect(store.visible(scope, 'side', view).filter((t) => t.kind === 'ext').length).toBe(0));
  });

  it('the ext tab renders its entry through EntryView', async () => {
    await openTracking();
    put({ id: 'ext/panel:notes', slot: 'sidePane', title: 'Notes', view: tree('the notes body') });
    render(() => (
      <ExtPaneTab
        tab={{ id: 't1', kind: 'ext', params: { entryId: 'ext/panel:notes', hostId: HOST, sessionId: 's1' } }}
        area="side"
        hostId={HOST}
        projectKey="~"
        sessionId="s1"
        close={() => {}}
        setTitle={() => {}}
      />
    ));
    await waitFor(() => expect(screen.getByText('the notes body')).toBeTruthy());
  });
});

// ── fullscreen page + command-palette "Open <title>" ────────────────────

describe('U10 fullscreen page + palette entries', () => {
  it('a fullscreen entry adds a rail page, a palette command and renders at /x/:id', async () => {
    await openTracking();
    const reg = sessionUi(HOST, 's1');
    render(() => (
      <Router>
        <Route
          path="/"
          component={() => {
            feedExtensionPages(() => reg);
            feedPaletteEntries({ hostId: HOST, sessionId: 's1', cwd: () => undefined, registry: () => reg });
            return <ExtensionPage entryId="util/page:usage" />;
          }}
        />
      </Router>
    ));
    put({ id: 'util/page:usage', slot: 'fullscreen', order: 5, title: 'Usage', view: tree('usage bars') });
    await waitFor(() => expect(railItems().some((i) => i.id === 'x:util/page:usage' && i.label === 'Usage')).toBe(true));
    const cmd = allCommands().find((c) => c.id === 'open-entry:util/page:usage');
    expect(cmd?.title).toBe('Open Usage');
    expect(cmd?.group).toBe('Panels');
    await waitFor(() => expect(screen.getByText('usage bars')).toBeTruthy());
    // deletion removes both live (§6.6)
    del('util/page:usage');
    await waitFor(() => expect(railItems().some((i) => i.id === 'x:util/page:usage')).toBe(false));
    expect(allCommands().some((c) => c.id === 'open-entry:util/page:usage')).toBe(false);
    await waitFor(() => expect(screen.getByTestId('ext-page-missing')).toBeTruthy());
  });

  it('sidePane/bottomPane entries get an "Open <title>" command too (§14 U10)', async () => {
    await openTracking();
    const reg = sessionUi(HOST, 's1');
    render(() => (
      <Router>
        <Route
          path="/"
          component={() => {
            feedPaletteEntries({ hostId: HOST, sessionId: 's1', cwd: () => undefined, registry: () => reg });
            return <p>host</p>;
          }}
        />
      </Router>
    ));
    put({ id: 'ext/panel:notes', slot: 'sidePane', title: 'Notes', view: tree('notes') });
    put({ id: 'ext/panel:build', slot: 'bottomPane', title: 'Build', view: tree('build') });
    await waitFor(() => expect(allCommands().find((c) => c.id === 'open-entry:ext/panel:notes')?.title).toBe('Open Notes'));
    expect(allCommands().find((c) => c.id === 'open-entry:ext/panel:build')?.title).toBe('Open Build');
    // running it opens the entry's tab in the side pane / bottom panel
    allCommands().find((c) => c.id === 'open-entry:ext/panel:build')!.run();
    await waitFor(() => expect(panes().visible({ hostId: HOST, projectKey: '~' }, 'bottom', { sessionId: 's1' }).some((t) => t.params?.entryId === 'ext/panel:build')).toBe(true));
  });
});

// ── phone: pane layer + the ⋯ "Panes" group (§13.6 row 6) ───────────────

describe('U10 fullscreen rail feed at the shell (survives leaving the chat)', () => {
  it('keeps the page in the rail while the session is open, drops it on close', async () => {
    await openTracking();
    const [path, setPath] = createSignal('/s/h1/s1');
    render(() => {
      installExtensionPageFeed(path);
      return <p>rail</p>;
    });
    put({ id: 'util/page:usage', slot: 'fullscreen', title: 'Usage', view: tree('usage') });
    await waitFor(() => expect(railItems().some((i) => i.id === 'x:util/page:usage')).toBe(true));
    // navigate to Settings: the conversation unmounts, the page's item stays
    setPath('/settings');
    await new Promise((r) => setTimeout(r, 10));
    expect(railItems().some((i) => i.id === 'x:util/page:usage')).toBe(true);
    // the entry is deleted → gone
    del('util/page:usage');
    await waitFor(() => expect(railItems().some((i) => i.id === 'x:util/page:usage')).toBe(false));
  });
});

describe('U10 phone pane layer', () => {
  it('lists sidePane/bottomPane entries in the Panes group and opens a full-screen layer', async () => {
    setDesktopOverride(false);
    await openTracking();
    const reg = sessionUi(HOST, 's1');
    put({ id: 'ext/panel:notes', slot: 'sidePane', title: 'Notes', view: tree('notes body') });
    render(() => (
      <div>
        <PhonePanesGroup registry={reg} onOpen={() => {}} />
        <PhonePaneLayer registry={reg} target={TARGET} />
      </div>
    ));
    fireEvent.click(await screen.findByText('Notes'));
    await waitFor(() => expect(screen.getByTestId('ui-phone-pane')).toBeTruthy());
    expect(screen.getByText('notes body')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Back to chat' }));
    await waitFor(() => expect(screen.queryByTestId('ui-phone-pane')).toBeNull());
  });
});

// ── turnFooter: inside the turn's row, per (entry, turn) (§6.7) ─────────

describe('U10 turn footers', () => {
  const ctxOf = (s: SessionHandle) =>
    ({ session: s, hostId: HOST, sessionId: () => 's1', columnWidth: () => 800 }) as unknown as TimelineCtx;
  const turnOf = (turnId: string, live = false): Turn => ({ turnId, kind: 'turn', entries: [], segments: [], live, stopped: false, source: 'user' });

  const renderCmds = () => world_.log.filter((e) => e.kind === 'cmd' && e.op === '_oino/render' && (e.args as { target?: string })?.target === 'turnFooter').map((e) => e.args);

  it('asks _oino/render per turn, renders the node and re-asks on invalidate', async () => {
    const s = await openTracking();
    put({ id: 'unipi.file-summary/turnFooter:files', slot: 'turnFooter', view: { kind: 'renderer', key: 'files' } });
    render(() => <TurnFooters ctx={ctxOf(s)} turn={turnOf('e-user-1')} />);
    await waitFor(() => expect(screen.getByText('edited src/ui/slots/WidgetStack.tsx · +84 −12')).toBeTruthy());
    expect(renderCmds().length).toBe(1);
    expect(renderCmds()[0]).toMatchObject({ type: '_oino/render', target: 'turnFooter', key: 'files', turnEntryId: 'e-user-1', width: 'normal' });
    // §6.7 invalidate: the rev bumps the cache key → one more render
    pushOps([{ op: 'invalidate', id: 'unipi.file-summary/turnFooter:files' }]);
    await waitFor(() => expect(renderCmds().length).toBe(2));
  });

  it('renders nothing while no renderer entry exists, and nothing for a null answer', async () => {
    const s = await openTracking();
    const first = render(() => <TurnFooters ctx={ctxOf(s)} turn={turnOf('e-user-1')} />);
    await new Promise((r) => setTimeout(r, 10));
    expect(screen.queryByTestId('ui-turnfooters')).toBeNull();
    first.unmount();
    // the mock answers `null` for an unknown key: nothing renders, never an error
    put({ id: 'ext/turnFooter:other', slot: 'turnFooter', view: { kind: 'renderer', key: 'other' } });
    render(() => <TurnFooters ctx={ctxOf(s)} turn={turnOf('e-user-1')} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByText(/edited/)).toBeNull();
  });
});
