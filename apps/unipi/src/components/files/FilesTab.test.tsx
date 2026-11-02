/**
 * Files tab tests (WP-A20): the tree over the mock host (lazy expand,
 * compact folders, git badges, changed filter), the row menu (rename,
 * delete, copy path, mention → the @path chip mailbox), search
 * (fs_search), upload by drop (blob_put + fs_import), the editor area
 * (load/save through the shared editor target) and the preview tab
 * (markdown ↔ source, image bytes). Acceptance: tree model tests live in
 * `lib/files/tree.test.ts`; these cover the wiring.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { JSX } from 'solid-js';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockFsNode, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetPanes } from '../../lib/panes';
import { paneKind } from '../../lib/panes/kinds';
import { clearEditorTarget, getEditorTarget, setEditorTarget } from '../../lib/editorTarget';
import { consumeMentionFor, pendingMention } from '../../lib/chat/mention';
import { FileTree } from './FileTree';
import { EditorArea } from './EditorArea';
import { FilesLayer } from './FilesTab';
import { setDesktopOverride } from '../../lib/layout';
import { PreviewTab } from '../media/Previewer';
import { setBlobTransport, type BlobTransport } from '../../lib/blob';
import type { PaneTabProps } from '../../lib/panes/kinds';

const HOST = 'local';
const ROOT = '/home/you/p';

const dir = (children: Record<string, MockFsNode>, extra: Partial<Extract<MockFsNode, { kind: 'dir' }>> = {}): MockFsNode => ({ kind: 'dir', children, ...extra });
const file = (text: string, extra: Partial<Extract<MockFsNode, { kind: 'file' }>> = {}): MockFsNode => ({ kind: 'file', text, ...extra });

const fs = (): Record<string, MockFsNode> => ({
  [ROOT]: dir({
    src: dir({
      lib: dir({ 'tree.ts': file('export {};\n') }),
      'math.ts': file('export function add(a: number, b: number) {\n  return a + b;\n}\n', { git: 'M' }),
    }),
    'README.md': file('# unipi\n\nThe Unipi app.\n', { git: 'A' }),
    'logo.svg': file('<svg xmlns="http://www.w3.org/2000/svg"></svg>\n'),
    'shot.png': file('png'),
    'notes.md': file('# Notes\n\nSome text about pineapples.\n'),
    target: dir({ debug: dir({}) }, { ignored: true }),
    '.env': file('SECRET=1\n'),
  }),
});

const cards = [
  { session_id: 's1', title: 'Fix the build', cwd: ROOT, last_activity: 1_700_000_000_000 },
  { session_id: 's2', title: 'Write docs', cwd: ROOT, last_activity: 1_690_000_000_000 },
];

const scenario: MockScenario = {
  name: 'files-test',
  hosts: [{ id: HOST, name: 'workstation', transport: 'local', cards: cards as never[], fs: fs() }],
};

let world: ReturnType<typeof createMockHost>;

beforeEach(() => {
  world = createMockHost({ ...scenario, hosts: [{ ...scenario.hosts[0]!, fs: fs() }] });
  setHostClientFactory(world.client);
  // The blob channel over the same mock host (uploads + preview bytes).
  setBlobTransport({
    async putBytes(hostId, bytes, mime, name) {
      return world.client(hostId).request('blob_put_bytes', { bytes: Array.from(bytes), mime, name });
    },
    async getBytes(hostId, ref) {
      return world.client(hostId).request('blob_get_bytes', ref);
    },
  } as Partial<BlobTransport> as BlobTransport);
  // jsdom has no blob URLs: a stable fake (only the tab's bytes matter here).
  URL.createObjectURL = ((blob: Blob) => 'blob:fake/' + blob.size) as typeof URL.createObjectURL;
  URL.revokeObjectURL = (() => {}) as typeof URL.revokeObjectURL;
  localStorage.clear();
  clearEditorTarget();
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  setBlobTransport(undefined);
  vi.restoreAllMocks();
});

const treeProps = (over: Partial<Parameters<typeof FileTree>[0]> = {}) => ({
  hostId: HOST,
  root: ROOT,
  roots: () => undefined,
  openFile: vi.fn(),
  onPreview: vi.fn(),
  ...over,
});

/** Renders inside a router: the tree navigates on mention picks and phone edits. */
function withRouter(body: () => JSX.Element) {
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => (
        <SettingsProvider>
          <HostsProvider>{body()}</HostsProvider>
        </SettingsProvider>
      )} />
    </HashRouter>
  ));
}

async function tree(over: Partial<Parameters<typeof FileTree>[0]> = {}) {
  const props = treeProps(over);
  const utils = withRouter(() => <FileTree {...props} />);
  await screen.findByRole('tree'); // the shell
  await screen.findAllByRole('treeitem'); // the root listing landed
  return { ...utils, props };
}

describe('FileTree', () => {
  it('registers the pane views (files + preview) with the kinds registry', async () => {
    await import('./FilesTab');
    expect(paneKind('files')?.component).toBeTruthy();
    expect(paneKind('preview')?.component).toBeTruthy();
  });

  it('lists the project root lazily: dirs first, dotfiles shown, git badges', async () => {
    await tree();
    const rows = screen.getAllByRole('treeitem');
    const names = rows.map((r) => r.getAttribute('data-test-row'));
    expect(names).toContain(`${ROOT}/src`);
    expect(names).toContain(`${ROOT}/.env`);
    expect(names.indexOf(`${ROOT}/src`)).toBeLessThan(names.indexOf(`${ROOT}/README.md`));
    // root-level badges first; `M` arrives with src's listing
    expect(screen.getByText('A', { selector: '.ftree-git' })).toBeTruthy();
    fireEvent.click(screen.getByText('src'));
    await waitFor(() => expect(screen.getByText('M', { selector: '.ftree-git' })).toBeTruthy());
    // gitignored rows carry the dim flag
    const zone = screen.getByTestId('files-tree');
    expect(zone.querySelector('[data-ignored]')).toBeTruthy();
  });

  it('expands a dir on click and compacts single-child chains (target/debug)', async () => {
    await tree();
    fireEvent.click(screen.getByText('src'));
    await waitFor(() => expect(screen.getByText('math.ts')).toBeTruthy());
    expect(screen.getByText('lib')).toBeTruthy();
    // target holds only `debug`: the chain compacts into one `target/debug`
    // row (keyed by debug's path), which expands to its (empty) listing.
    fireEvent.click(screen.getByText('target'));
    const compact = await screen.findByText('target/debug');
    expect(compact.closest('.ftree-row')!.getAttribute('data-test-row')).toBe(`${ROOT}/target/debug`);
    fireEvent.click(compact);
    await waitFor(() => expect(screen.getByText('(empty)')).toBeTruthy());
  });

  it('opens a text file through openFile and media through onPreview with siblings', async () => {
    const { props } = await tree();
    fireEvent.click(screen.getByText('src'));
    await waitFor(() => screen.getByText('math.ts'));
    fireEvent.click(screen.getByText('math.ts'));
    expect(props.openFile).toHaveBeenCalledWith(`${ROOT}/src/math.ts`, undefined);
    fireEvent.click(screen.getByText('shot.png'));
    expect(props.onPreview).toHaveBeenCalledWith(`${ROOT}/shot.png`, [`${ROOT}/logo.svg`, `${ROOT}/shot.png`]);
  });

  it('changed filter shows only git-marked entries as relative paths', async () => {
    await tree();
    fireEvent.click(screen.getByText('src'));
    await waitFor(() => expect(screen.getByText('math.ts')).toBeTruthy());
    fireEvent.click(screen.getByTestId('files-changed'));
    await waitFor(() => {
      const rows = screen.getAllByRole('treeitem').map((r) => r.getAttribute('data-test-row'));
      expect(rows).toEqual([`${ROOT}/README.md`, `${ROOT}/src/math.ts`]);
    });
  });

  it('row menu: a vertical menu with keyboard nav; Escape closes it', async () => {
    await tree();
    fireEvent.contextMenu(screen.getByText('README.md'));
    const menu = await screen.findByRole('menu');
    const items = Array.from(menu.querySelectorAll('[role=menuitem]'));
    expect(items.map((i) => i.textContent)).toEqual(['Edit as text', 'Rename…', 'Copy path', 'Mention in chat', 'Delete…']);
    // Every item is one row: its icon sits beside the label in the same button.
    for (const item of items) expect(item.classList.contains('board-menu-row')).toBe(true);
    await waitFor(() => expect(document.activeElement).toBe(items[0]));
    fireEvent.keyDown(menu, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(items[1]);
    fireEvent.keyDown(menu, { key: 'ArrowUp' });
    fireEvent.keyDown(menu, { key: 'ArrowUp' });
    expect(document.activeElement).toBe(items[4]);
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
  });

  it('row menu: rename is inline in the tree row', async () => {
    await tree();
    fireEvent.click(screen.getByText('src'));
    await waitFor(() => screen.getByText('math.ts'));
    fireEvent.contextMenu(screen.getByText('math.ts'));
    await screen.findByRole('menu');
    fireEvent.click(screen.getByText('Rename…'));
    const input = (await screen.findByTestId('files-rename-input')) as HTMLInputElement;
    expect(input.closest('[role=treeitem]')?.getAttribute('data-test-row')).toBe(`${ROOT}/src/math.ts`);
    input.value = 'math2.ts';
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(screen.getByText('Renamed to math2.ts')).toBeTruthy());
    await waitFor(() => expect(screen.getByText('math2.ts')).toBeTruthy());
    expect(world.log.some((e) => e.op === 'fs_rename' && (e.args as { to?: string }).to === `${ROOT}/src/math2.ts`)).toBe(true);
  });

  it('row menu: delete asks, then removes the row', async () => {
    await tree();
    fireEvent.contextMenu(screen.getByText('README.md'));
    await screen.findByRole('menu');
    fireEvent.click(screen.getByText('Delete…'));
    fireEvent.click(await screen.findByTestId('files-delete-confirm'));
    await waitFor(() => expect(screen.queryByText('README.md')).toBeNull());
    expect(world.log.some((e) => e.op === 'fs_delete')).toBe(true);
  });

  it('row menu: copy path puts the path on the clipboard', async () => {
    const write = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText: write } });
    await tree();
    fireEvent.click(screen.getByText('src'));
    await waitFor(() => screen.getByText('math.ts'));
    fireEvent.contextMenu(screen.getByText('math.ts'));
    await screen.findByRole('menu');
    fireEvent.click(screen.getByText('Copy path'));
    await waitFor(() => expect(write).toHaveBeenCalledWith(`${ROOT}/src/math.ts`));
  });

  it('Mention in chat queues an @path chip for the picked chat (acceptance)', async () => {
    const { props } = await tree();
    fireEvent.click(screen.getByText('src'));
    await waitFor(() => screen.getByText('math.ts'));
    fireEvent.contextMenu(screen.getByText('math.ts'));
    await screen.findByRole('menu');
    fireEvent.click(screen.getByTestId('files-mention'));
    const row = await screen.findAllByTestId('chat-pick-row');
    fireEvent.click(row[0]!);
    expect(pendingMention()).toEqual({ key: `${HOST}:s1`, text: `@${ROOT}/src/math.ts ` });
    expect(consumeMentionFor(`${HOST}:s1`)).toBe(`@${ROOT}/src/math.ts `);
    expect(props.openFile).not.toHaveBeenCalled();
  });

  it('search: fs_search name results ranked fuzzy, content with lines; picking opens the file', async () => {
    const { props } = await tree();
    fireEvent.click(screen.getByTestId('files-search'));
    const input = screen.getByTestId('files-search-input');
    // Live search: typing runs the query (150 ms debounce), no button.
    fireEvent.input(input, { target: { value: 'math' } });
    await waitFor(() => expect(screen.getAllByTestId('files-search-row').length).toBeGreaterThan(0));
    expect(screen.getByText('math.ts')).toBeTruthy();
    // content tab: the query hits line 2
    fireEvent.click(screen.getByTestId('files-search-content'));
    fireEvent.input(input, { target: { value: 'return a + b' } });
    await waitFor(() => expect(screen.getByText(':2')).toBeTruthy());
    fireEvent.click(screen.getAllByTestId('files-search-row')[0]!);
    expect(props.openFile).toHaveBeenCalledWith(`${ROOT}/src/math.ts`, { line: 2 });
  });

  it('upload by drop: blob_put_bytes then fs_import into the project root', async () => {
    await tree();
    const zone = screen.getByTestId('files-tree');
    const dropped = new File([new Uint8Array([104, 105])], 'hello.txt', { type: 'text/plain' });
    fireEvent.drop(zone, { dataTransfer: { types: ['Files'], files: [dropped] } });
    await waitFor(() => expect(screen.getByText('Uploaded hello.txt')).toBeTruthy());
    const ops = world.log.map((e) => e.op);
    expect(ops).toContain('blob_put_bytes');
    expect(world.log.some((e) => e.op === 'fs_import' && (e.args as { to?: string }).to === `${ROOT}/hello.txt`)).toBe(true);
    await waitFor(() => expect(screen.getByText('hello.txt')).toBeTruthy());
  });

  it('refreshes listings when the window regains focus (watchers)', async () => {
    await tree();
    expect(screen.queryByText('NEW.md')).toBeNull();
    // Another writer (the agent) drops a file into the project behind the app's back.
    await world.client(HOST).request('fs_write', { path: `${ROOT}/NEW.md`, text: 'new\n', create: true });
    window.dispatchEvent(new Event('focus'));
    await waitFor(() => expect(screen.getByText('NEW.md')).toBeTruthy());
  });
});

describe('EditorArea (the tree editor host)', () => {
  it('loads the editor target, edits mark dirty, save writes with the base hash', async () => {
    setEditorTarget({ hostId: HOST, path: `${ROOT}/src/math.ts` });
    withRouter(() => <EditorArea onBinary={vi.fn()} />);
    await waitFor(() => expect(screen.getByText('math.ts')).toBeTruthy());
    await waitFor(() => expect(document.querySelector('.cm-content')).toBeTruthy());
    const { EditorView } = await import('@codemirror/view');
    const view = EditorView.findFromDOM(document.querySelector('.cm-editor') as HTMLElement)!;
    view.dispatch({ changes: { from: 5, insert: '!' } });
    await waitFor(() => expect((screen.getByTestId('files-editor-save') as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByTestId('files-editor-save'));
    await waitFor(() => expect(world.log.some((e) => e.op === 'fs_write' && (e.args as { path?: string }).path === `${ROOT}/src/math.ts`)).toBe(true));
  });

  it('markdown targets open with the preview showing, toggling reveals source', async () => {
    setEditorTarget({ hostId: HOST, path: `${ROOT}/README.md` });
    withRouter(() => <EditorArea onBinary={vi.fn()} />);
    await waitFor(() => expect(screen.getByTestId('files-editor-md')).toBeTruthy());
    expect(screen.getByTestId('files-editor-md').textContent).toContain('The Unipi app.');
    fireEvent.click(screen.getByTestId('files-editor-preview'));
    await waitFor(() => expect(screen.queryByTestId('files-editor-md')).toBeNull());
    // the editor mounts async (its language loads first)
    await waitFor(() => expect(document.querySelector('.cm-editor')).toBeTruthy());
  });

  it('binary files route to the preview instead', async () => {
    const onBinary = vi.fn();
    setEditorTarget({ hostId: HOST, path: `${ROOT}/shot.png` });
    withRouter(() => <EditorArea onBinary={onBinary} />);
    await waitFor(() => expect(onBinary).toHaveBeenCalledWith(`${ROOT}/shot.png`));
  });
});

describe('PreviewTab (Z-103)', () => {
  const tabProps = (path: string): PaneTabProps => ({
    tab: { id: 'p', kind: 'preview', params: { hostId: HOST, path } },
    area: 'side',
    hostId: HOST,
    projectKey: ROOT,
    close: vi.fn(),
    setTitle: vi.fn(),
  });

  it('renders markdown, toggles to source and back', async () => {
    render(() => <PreviewTab {...tabProps(`${ROOT}/README.md`)} />);
    await waitFor(() => expect(screen.getByTestId('preview-tab').textContent).toContain('The Unipi app.'));
    fireEvent.click(screen.getByTestId('preview-tab-source'));
    await waitFor(() => expect(screen.getByTestId('preview-tab').textContent).toContain('# unipi'));
  });

  it('renders images from blob bytes (checkerboard stage)', async () => {
    render(() => <PreviewTab {...tabProps(`${ROOT}/shot.png`)} />);
    const img = await screen.findByTestId('preview-tab-image');
    expect(img.getAttribute('src')).toMatch(/^blob:/);
  });

  it('svg renders as an image and shows its markup in source mode', async () => {
    render(() => <PreviewTab {...tabProps(`${ROOT}/logo.svg`)} />);
    await screen.findByTestId('preview-tab-image');
    fireEvent.click(screen.getByTestId('preview-tab-source'));
    await waitFor(() => expect(screen.getByTestId('preview-tab').textContent).toContain('<svg'));
  });
});

describe('phone Files tab (§3.11)', () => {
  const phone = (hash: string) => {
    setDesktopOverride(false);
    location.hash = hash;
    return withRouter(() => <FilesLayer />);
  };
  afterEach(() => setDesktopOverride(undefined));

  it('the root is Places — Home, then the PC\'s projects (PH-24)', async () => {
    phone('#/files');
    await screen.findByTestId('files-places');
    expect(await screen.findByTestId('files-place-home')).toBeTruthy();
    expect(screen.queryByRole('tree')).toBeNull();
  });

  it('a folder is a sans list: folders first, no dotfiles by default, size · date subtitle', async () => {
    phone(`#/files?path=${encodeURIComponent(ROOT)}&root=${encodeURIComponent(ROOT)}`);
    const rows = await screen.findAllByTestId('files-row');
    const names = rows.map((r) => r.querySelector('.phfiles-name')?.textContent);
    expect(names.slice(0, 2)).toEqual(['src', 'target']);
    expect(names).toContain('README.md');
    expect(names).not.toContain('.env');
    // Every row has its ⋯ (always visible, PH-37).
    expect(screen.getAllByTestId('files-row-menu').length).toBe(rows.length);
  });

  it('tapping a text file opens the viewer route above the folder (PH-02)', async () => {
    phone(`#/files?path=${encodeURIComponent(ROOT)}`);
    const rows = await screen.findAllByTestId('files-row');
    fireEvent.click(rows.find((r) => r.textContent?.includes('notes.md'))!);
    await waitFor(() => expect(location.hash).toContain('#/files/edit'));
    expect(getEditorTarget()?.path).toBe(`${ROOT}/notes.md`);
  });

  it('the row ⋯ opens an action sheet (PH-10)', async () => {
    phone(`#/files?path=${encodeURIComponent(ROOT)}`);
    const menus = await screen.findAllByTestId('files-row-menu');
    fireEvent.click(menus[0]!);
    const sheet = await screen.findByTestId('files-row-sheet');
    expect(sheet.textContent).toContain('Rename');
    expect(sheet.textContent).toContain('Copy path');
    expect(sheet.textContent).toContain('Delete');
  });
});

afterEach(() => {
  resetPanes();
});
