// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ProcessGroup } from './Transcript';
import type { ToolRun } from '../../lib/chat/model';
import type { Chat } from '../../lib/chat/store';

const openFilePath = vi.fn(async (..._a: unknown[]) => {});
vi.mock('../../lib/media/pathTokens', async (orig) => ({
  ...(await orig<typeof import('../../lib/media/pathTokens')>()),
  openFilePath: (...a: unknown[]) => openFilePath(...a),
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const CWD = '/home/u/proj';
const EXIST = new Set([`${CWD}/src/a.ts`, `${CWD}/packages/fusion/src/picker.ts`, `${CWD}/README.md`]);
const mkChat = () =>
  ({
    hostId: 'h1',
    session: '1',
    state: { session: { cwd: CWD } },
    statPaths: vi.fn(async (paths: string[]) =>
      paths.map((path) => {
        const resolved = path.startsWith('/') ? path : `${CWD}/${path}`;
        return { path, resolved, kind: EXIST.has(resolved) ? 'file' : 'missing' };
      }),
    ),
  }) as unknown as Chat;

const mount = (tool: ToolRun, chat: Chat) =>
  render(() => (
    <HashRouter root={(p) => p.children}>
      <Route path="/" component={() => <ProcessGroup items={[{ kind: 'tool', key: 't1', tool }]} live={false} cwd={CWD} chat={chat} />} />
    </HashRouter>
  ));

describe('tool rows: file subjects and output are tappable (UNI-204)', () => {
  it("a read row's subject is a link to the verified file; tapping it opens without expanding", async () => {
    const chat = mkChat();
    mount({ id: '1', name: 'read', args: { path: `${CWD}/src/a.ts` }, state: 'ok', output: 'file contents' }, chat);
    fireEvent.click(screen.getByText('Read files'));
    await waitFor(() => expect(document.querySelector('.tool-subject .file-ref')).toBeTruthy());
    const row = document.querySelector('.tool-row')!;
    fireEvent.click(document.querySelector('.tool-subject .file-ref')!);
    expect(openFilePath).toHaveBeenCalledWith(chat, `${CWD}/src/a.ts`);
    expect(row.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(screen.getByText('read')); // the rest of the row still expands
    expect(row.getAttribute('aria-expanded')).toBe('true');
  });

  it('a subject path that does not exist stays plain', async () => {
    const chat = mkChat();
    mount({ id: '1', name: 'read', args: { path: 'src/gone.ts' }, state: 'ok', output: '' }, chat);
    fireEvent.click(screen.getByText('Read files'));
    await waitFor(() => expect(chat.statPaths).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 10));
    expect(document.querySelector('.tool-subject .file-ref')).toBeNull();
  });

  it('grep output lines link to files relative to the grep path, with the line', async () => {
    const chat = mkChat();
    mount({ id: '1', name: 'grep', args: { path: 'packages/fusion/src/picker.ts', pattern: 'lead' }, state: 'ok', output: 'picker.ts:15:  * lead\npicker.ts:24:  * sidekick' }, chat);
    fireEvent.click(screen.getByText('Searched'));
    fireEvent.click(screen.getByText('grep'));
    await waitFor(() => expect(document.querySelectorAll('.tool-read .file-ref')).toHaveLength(2));
    const first = document.querySelector<HTMLElement>('.tool-read .file-ref')!;
    expect(first.textContent).toBe('picker.ts:15');
    expect(first.dataset.fileLine).toBe('15');
    expect(first.dataset.fileRef).toBe(`${CWD}/packages/fusion/src/picker.ts`);
  });

  it('bash output: existing paths link, the command and missing ones stay plain', async () => {
    const chat = mkChat();
    mount({ id: '1', name: 'bash', args: { command: 'git status --short' }, state: 'ok', output: ' M src/a.ts\n?? src/new.ts\n M README.md' }, chat);
    fireEvent.click(screen.getByText('Ran commands'));
    fireEvent.click(screen.getByText('bash'));
    await waitFor(() => expect(document.querySelectorAll('.tool-term-out .file-ref')).toHaveLength(2));
    expect([...document.querySelectorAll('.tool-term-out .file-ref')].map((e) => e.textContent)).toEqual(['src/a.ts', 'README.md']);
    expect(document.querySelector('.tool-term-out')!.textContent).toBe(' M src/a.ts\n?? src/new.ts\n M README.md');
  });
});

describe('older pi without paths_stat (UNI-204 regression)', () => {
  const oldPi = () => ({ hostId: 'h1', session: '2', state: { session: { cwd: CWD } }, statPaths: vi.fn(async () => null) }) as unknown as Chat;

  it('a read row keeps its Open: the subject links unverified, resolved against the cwd', async () => {
    const chat = oldPi();
    mount({ id: '1', name: 'read', args: { path: 'src/a.ts' }, state: 'ok', output: 'x' }, chat);
    fireEvent.click(screen.getByText('Read files'));
    await waitFor(() => expect(document.querySelector('.tool-subject .file-ref')).toBeTruthy());
    expect(document.querySelector<HTMLElement>('.tool-subject .file-ref')!.dataset.fileRef).toBe(`${CWD}/src/a.ts`);
    fireEvent.click(document.querySelector('.tool-subject .file-ref')!);
    expect(openFilePath).toHaveBeenCalledWith(chat, `${CWD}/src/a.ts`);
  });

  it('an edit row with an absolute path links too', async () => {
    const chat = oldPi();
    mount({ id: '1', name: 'edit', args: { path: '/etc/hosts', oldText: 'a', newText: 'b' }, state: 'ok', output: '' }, chat);
    fireEvent.click(screen.getByText(/Edited|Changed|Edit/));
    await waitFor(() => expect(document.querySelector<HTMLElement>('.tool-subject .file-ref')?.dataset.fileRef).toBe('/etc/hosts'));
  });

  it('absolute paths in tool output link unverified; relative ones stay plain', async () => {
    const chat = oldPi();
    mount({ id: '1', name: 'bash', args: { command: 'ls' }, state: 'ok', output: '/home/u/proj/README.md\nsrc/a.ts' }, chat);
    fireEvent.click(screen.getByText('Ran commands'));
    fireEvent.click(screen.getByText('bash'));
    await waitFor(() => expect(document.querySelectorAll('.tool-term-out .file-ref')).toHaveLength(1));
    expect(document.querySelector<HTMLElement>('.tool-term-out .file-ref')!.dataset.fileRef).toBe('/home/u/proj/README.md');
  });

  it('a newer pi that answers missing still keeps the subject plain', async () => {
    const chat = mkChat();
    mount({ id: '1', name: 'read', args: { path: 'src/gone.ts' }, state: 'ok', output: '' }, chat);
    fireEvent.click(screen.getByText('Read files'));
    await waitFor(() => expect(chat.statPaths).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 10));
    expect(document.querySelector('.tool-subject .file-ref')).toBeNull();
  });
});
