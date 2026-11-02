/**
 * Files-tab scenarios (WP-A20, Z-101 file tree / Z-103 preview pane):
 *
 *   files-tree     the `files` side-pane tab on a real-looking project:
 *                  editor left (an open `tree.ts`), tree right with compact
 *                  folders, dotfiles, git badges and an ignored folder
 *   files-search   the same tab with the search panel open on a content
 *                  query (`fs_search kind:content`)
 *   files-preview  the `preview` tab on an SVG (checkerboard stage, source
 *                  toggle in the bar); the phone gets the files layer and
 *                  the previewer layer for the same file
 *
 * The tree is the unipi repo itself (shallow), with git marks so the filter
 * and the badges have something to show. `setup` seeds the shell + pane state
 * and, where a capture needs an interaction (expanded folders, the search
 * panel, the phone preview layer), drives the real DOM once it is mounted —
 * the same way `git.ts` opens the Git layer on the phone.
 */
import type { MockFsNode, MockScenario } from '../host';
import { genCards, genSession } from '../gen';
import { openPreviewer } from '../../../lib/media/previewerState';

const HOST = 'this-pc';
const CWD = '/home/you/projects/unipi';

const f = (text: string, git?: 'M' | 'A' | 'D' | '?'): MockFsNode => ({ kind: 'file', text, mtime: 1_761_000_000_000, ...(git ? { git } : {}) });
const d = (children: Record<string, MockFsNode>, extra: { git?: 'M' | 'A' | 'D' | '?'; ignored?: boolean } = {}): MockFsNode => ({ kind: 'dir', children, ...extra });

const TREE_TS = `/**
 * Files tree model (WP-A20): a lazy, virtualized tree over \`fs_list\`.
 * Pure data + pure functions so the shape is testable without a host.
 */
import { joinPath } from '../fs';

export type TreeEntry = {
  name: string;
  kind: 'file' | 'dir' | 'symlink';
  size: number;
  mtime: number;
  git?: GitMark;
  ignored?: boolean;
};

export type DirState = {
  status: 'idle' | 'loading' | 'ready' | 'error';
  entries?: TreeEntry[];
  error?: string;
  more?: boolean;
};

/** Compact folders: a chain of single-child dirs renders as one \`a/b\` row. */
function compactible(model: TreeModel, path: string): string | undefined {
  const state = model.dirs.get(path);
  if (!state || state.status !== 'ready' || !state.entries) return undefined;
  const entries = sortEntries(state.entries);
  if (entries.length !== 1 || entries[0]!.kind !== 'dir') return undefined;
  return joinPath(path, entries[0]!.name);
}

/** Pure: the visible rows, depth-first, compact chains merged. */
export function flatten(model: TreeModel): TreeRow[] {
  const rows: TreeRow[] = [];
  const walk = (path: string, depth: number) => {
    const state = model.dirs.get(path);
    for (const entry of sortEntries(state?.entries ?? [])) {
      const child = joinPath(path, entry.name);
      const open = model.expanded.has(child);
      rows.push({ path: child, name: entry.name, depth, kind: entry.kind, git: entry.git, expanded: open });
      if (open) walk(child, depth + 1);
    }
  };
  for (const root of model.roots) {
    rows.push({ path: root, name: root.split('/').pop()!, depth: 0, kind: 'root' });
    walk(root, 1);
  }
  return rows;
}
`;

const SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="120" viewBox="0 0 320 120">
  <g fill="none" stroke="#e05a3d" stroke-width="7" stroke-linecap="round">
    <path d="M38 44h30l-5 23a12 12 0 0 1-12 10h-6a12 12 0 0 1-12-10z"/>
    <path d="M29 38c-4-8 4-14 8-8M49 34c0-9 10-11 12-2M69 38c6-8 14-2 9 6"/>
    <path d="M88 46c-13 0-20 9-20 20s7 18 20 18"/>
  </g>
  <text x="120" y="80" font-family="Georgia, serif" font-size="54" fill="#f1ece4">unipi</text>
</svg>
`;

const FS: Record<string, MockFsNode> = {
  [CWD]: d({
    src: d({
      components: d({
        files: d({
          'FilesTab.tsx': f('export function FilesTab() {}\n', 'A'),
          'FileTree.tsx': f("import { flatten } from '../../lib/files/tree';\n", 'M'),
          'TreeRow.tsx': f('export function TreeRowView() {}\n'),
          'FileSearch.tsx': f('export function FileSearch() {}\n', 'A'),
          'files.css': f('.ftree-row { height: 26px; }\n', 'M'),
        }),
        shell: d({
          'AppShell.tsx': f('export function AppShell() {}\n', 'M'),
          'Sidebar.tsx': f('export function Sidebar() {}\n'),
        }),
      }),
      lib: d({
        files: d({ 'tree.ts': f(TREE_TS, 'M'), 'tree.test.ts': f("import { flatten } from './tree';\n"), 'watch.ts': f('export function createDirWatch() {}\n', 'A') }),
        session: d({ 'store.ts': f('export function createSessionStore() {}\n', 'M'), 'hub.ts': f('export function createHub() {}\n') }),
        'fs.ts': f('export async function fsList(hostId: string, path: string) {}\n', 'M'),
        'panes.ts': f('export function createPanes() {}\n'),
      }),
      ui: d({ 'registry.ts': f('export function createUiRegistry() {}\n'), 'NodeView.tsx': f('export function NodeView() {}\n') }),
    }),
    'extensions': d({
      unipi: d({
        'file-summary': d({ 'extension.json': f('{ "id": "unipi.file-summary" }\n'), src: d({ 'lib.rs': f('pub fn summarize() {}\n', 'M') }) }),
      }),
    }),
    docs: d({ 'dev': d({ 'v2': d({ 'DIRECTION.md': f('# Direction\n'), 'RULINGS.md': f('# Rulings\n'), specs: d({ 'app.md': f('# App\n', 'M') }) }) }) }),
    assets: d({ 'unicrab.svg': f(SVG, 'A'), 'logo.png': f('png', 'A') }),
    '.github': d({ workflows: d({ 'ci.yml': f('name: ci\n') }) }),
    'target': d({ debug: d({ 'unipi': f('bin', '?') }) }, { ignored: true }),
    '.env.local': f('OINO_HOME=/home/you/.oino\n'),
    '.gitignore': f('target/\nnode_modules/\ndist/\n'),
    'README.md': f('# unipi\n\nThe Unipi app on the oino host.\n', 'A'),
    'package.json': f('{ "name": "unipi", "private": true }\n'),
    'tsconfig.json': f('{ "strict": true }\n', 'M'),
    'vite.config.ts': f('export default {}\n'),
  }),
};

const cards = genCards(4, { seed: 7 }).map((c, i) => ({ ...c, cwd: CWD, session_id: `f${i + 1}` }));
const session = genSession('f1', { seed: 7, turns: 2, toolsPerTurn: 1, cwd: CWD });

const PANES = (active: string, extra: unknown[] = []) => ({
  side: {
    tabs: [{ id: 'tf', kind: 'files', params: { hostId: HOST, path: `${CWD}/src/lib/files/tree.ts` } }, ...extra],
    active: { f1: active },
    closed: [],
  },
  bottom: { tabs: [], active: {}, closed: [] },
});

function seed(sideOpen = true, sideW = 780, tabs: unknown = PANES('tf')): void {
  localStorage.setItem(
    'unipi.shell.layout',
    JSON.stringify({ sidebarOpen: false, sidebarW: 264, sideOpen, sideW, bottomOpen: false, bottomH: 220 }),
  );
  localStorage.setItem('unipi.shell.last', JSON.stringify({ hostId: HOST, sessionId: 'f1' }));
  if (tabs) localStorage.setItem(`unipi.panes:${HOST}|${CWD}`, JSON.stringify(tabs));
}

/** Runs `step` until it returns true (the app has mounted the thing being driven). */
function until(step: () => boolean, tries = 600): void {
  const tick = (left: number) => {
    if (step() || left <= 0) return;
    setTimeout(() => tick(left - 1), 100);
  };
  setTimeout(() => tick(tries), 120);
}

const row = (path: string): HTMLElement | null => document.querySelector(`[data-test-row="${path}"]`) as HTMLElement | null;

/** Expands a chain of folders, one per retry (each listing is lazy). */
function expand(...paths: string[]): void {
  let at = 0;
  until(() => {
    const el = row(paths[at]!);
    if (!el) return false;
    el.click();
    at += 1;
    return at >= paths.length;
  });
}

/** The shell the capture asked for (`?shell=`), so routing is decided before layout measures. */
function phoneShell(): boolean {
  const q = location.hash.indexOf('?');
  const params = new URLSearchParams(location.search);
  const fromHash = q >= 0 ? new URLSearchParams(location.hash.slice(q + 1)) : undefined;
  return (params.get('shell') ?? fromHash?.get('shell')) === 'phone';
}

function openFiles(): void {
  if (phoneShell() && !location.hash.startsWith('#/files')) location.hash = '#/files';
}

const hosts = () => [{ id: HOST, name: 'workstation', transport: 'local' as const, conn: { state: 'connected' as const }, cards, fs: FS, sessions: { f1: session } }];

export const filesTree: MockScenario = {
  name: 'files-tree',
  hosts: hosts(),
  setup: () => {
    seed();
    openFiles();
    expand(`${CWD}/src`, `${CWD}/src/lib`, `${CWD}/src/lib/session`);
  },
};

export const filesSearch: MockScenario = {
  name: 'files-search',
  hosts: hosts(),
  setup: () => {
    seed();
    openFiles();
    // Expand `src` so the tree behind the panel has content too, then drive
    // the real toolbar + panel: content search for "export".
    expand(`${CWD}/src`, `${CWD}/src/lib`);
    until(() => {
      const open = document.querySelector('[data-testid="files-search"]') as HTMLElement | null;
      if (!open) return false;
      open.click();
      const input = document.querySelector('[data-testid="files-search-input"]') as HTMLInputElement | null;
      if (!input) return false;
      (document.querySelector('[data-testid="files-search-content"]') as HTMLElement | null)?.click();
      if (input.value !== 'export') {
        input.value = 'export';
        input.dispatchEvent(new Event('input', { bubbles: true }));
      }
      // Live search: wait for the debounce + fs_search to land.
      return document.querySelectorAll('[data-testid="files-search-row"]').length > 3;
    });
  },
};

export const filesPreview: MockScenario = {
  name: 'files-preview',
  hosts: hosts(),
  setup: () => {
    seed(true, 640, PANES('tp', [{ id: 'tp', kind: 'preview', params: { hostId: HOST, path: `${CWD}/assets/unicrab.svg` } }]));
    openFiles();
    // Phone: the preview tab is the full-screen previewer layer (Z-103/§3.9).
    setTimeout(() => {
      if (!phoneShell()) return;
      openPreviewer({ kind: 'list', hostId: HOST, paths: [`${CWD}/assets/unicrab.svg`, `${CWD}/assets/logo.png`], index: 0 });
    }, 400);
  },
};

/** The editor scenarios (WP-A21): `editor-lsp` opens `tree.ts` with the
 *  scripted mock server — attach → didOpen → diagnostics badge on the tab
 *  + the tree row, `LspStatus` shows `ready`; `editor-conflict` opens the
 *  same file but the mock `E_LSP_TAKEN`s the attach 1.2 s later — the
 *  chip shows `taken` (clicking it takes the attach back). */
const lspHosts = (taken?: boolean) => [
  {
    id: HOST,
    name: 'workstation',
    transport: 'local' as const,
    conn: { state: 'connected' as const },
    cards,
    fs: FS,
    sessions: { f1: session },
    lsp: {
      name: 'typescript-language-server',
      diagnostics: [
        { severity: 1, message: "'mockUnused' is declared but never used" },
        { severity: 2, message: 'mock warning — an unused import' },
      ],
      taken,
    },
  },
];

export const editorLsp: MockScenario = {
  name: 'editor-lsp',
  hosts: lspHosts(),
  setup: () => {
    seed();
    openFiles();
    expand(`${CWD}/src`, `${CWD}/src/lib`, `${CWD}/src/lib/files`);
  },
};

export const editorConflict: MockScenario = {
  name: 'editor-conflict',
  hosts: lspHosts(true),
  setup: () => {
    seed();
    openFiles();
    expand(`${CWD}/src`, `${CWD}/src/lib`, `${CWD}/src/lib/files`);
  },
};
