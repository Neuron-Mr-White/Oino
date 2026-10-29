// @vitest-environment jsdom
/** Editor: unsaved changes ask before leaving (UNI-189 rework); toolbar layout. */
import { render, screen, waitFor, cleanup } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach, beforeEach } from 'vitest';
import Editor from './Editor';
import { HostsProvider } from '../hosts/context';
import { SettingsProvider } from '../settings/context';
import { call } from '../lib/ipc';
import { setEditorTarget } from '../lib/editorTarget';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const hosts = [{ id: 'host-1', name: 'coffee', ticket: 't', paired_at: 0, last_path: 'lan' as const }];

beforeEach(() => {
  vi.mocked(call).mockImplementation(async (cmd: string) => {
    if (cmd === 'list_hosts') return { ok: true, value: hosts };
    if (cmd === 'fs_read') return { ok: true, value: { size: 5, hash: 'h1', mtime: 1, truncated: false, text: 'hello' } };
    if (cmd === 'fs_write') return { ok: true, value: { hash: 'h2', mtime: 2, size: 6 } };
    return { ok: true, value: null };
  });
  setEditorTarget({ hostId: 'host-1', path: '/home/u/a.txt' });
  location.hash = '#/files/edit';
});
afterEach(() => {
  cleanup();
  vi.resetAllMocks();
  location.hash = '#/';
});

function mount() {
  return render(() => (
    <HashRouter>
      <Route path="*" component={() => <SettingsProvider><HostsProvider><Editor /></HostsProvider></SettingsProvider>} />
    </HashRouter>
  ));
}

async function typeSomething() {
  await waitFor(() => expect(document.querySelector('.cm-content')).toBeTruthy());
  const { EditorView } = await import('@codemirror/view');
  const view = EditorView.findFromDOM(document.querySelector('.cm-editor') as HTMLElement)!;
  view.dispatch({ changes: { from: 5, insert: '!' } });
}

describe('Editor leave guard', () => {
  it('Back with unsaved changes asks; Discard leaves without writing', async () => {
    mount();
    await typeSomething();
    screen.getByLabelText('Back to files').click();
    const sheet = await screen.findByTestId('editor-leave-sheet');
    expect(location.hash).toBe('#/files/edit');
    [...sheet.querySelectorAll('button')].find((b) => b.textContent === 'Discard')!.click();
    await waitFor(() => expect(location.hash).toBe('#/files'));
    expect(vi.mocked(call).mock.calls.some(([cmd]) => cmd === 'fs_write')).toBe(false);
  });

  it('Save in the prompt writes, then leaves', async () => {
    mount();
    await typeSomething();
    screen.getByLabelText('Back to files').click();
    const sheet = await screen.findByTestId('editor-leave-sheet');
    [...sheet.querySelectorAll('button')].find((b) => b.textContent === 'Save')!.click();
    await waitFor(() => expect(location.hash).toBe('#/files'));
    expect(vi.mocked(call).mock.calls.some(([cmd, args]) => cmd === 'fs_write' && (args as { text: string }).text === 'hello!')).toBe(true);
  });

  it('a clean file leaves straight away', async () => {
    mount();
    await waitFor(() => expect(document.querySelector('.cm-content')).toBeTruthy());
    screen.getByLabelText('Back to files').click();
    await waitFor(() => expect(location.hash).toBe('#/files'));
  });

  it('a very long file name is one truncating line (full name on hover); the unsaved dot stays visible', async () => {
    const long = `${'very-long-name-'.repeat(12)}.md`;
    setEditorTarget({ hostId: 'host-1', path: `/home/u/${long}` });
    mount();
    await typeSomething();
    const name = document.querySelector('.editor-title-name') as HTMLElement;
    expect(name.title).toBe(long);
    expect(name.querySelector('.editor-title-text')!.textContent).toBe(long);
    // The dot is a sibling of the truncating text, not inside it.
    await waitFor(() => expect(name.querySelector(':scope > .editor-dirty-dot')).toBeTruthy());
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const css = readFileSync(resolve(__dirname, 'files.css'), 'utf8');
    expect(css).toMatch(/\.editor-title-text \{[^}]*text-overflow: ellipsis[^}]*white-space: nowrap/);
  });

  it('Find and Save sit in the pinned end group, outside the scrolling keys', async () => {
    mount();
    await waitFor(() => expect(document.querySelector('.editor-toolbar')).toBeTruthy());
    const end = document.querySelector('.editor-toolbar-end')!;
    expect(end.textContent).toContain('Find');
    expect(end.textContent).toContain('Save');
    expect(document.querySelector('.editor-toolbar-keys')!.textContent).not.toContain('Save');
  });
});

describe('UNI-265: Markdown preview', () => {
  it('a .md file opens rendered; Edit shows the editor; Preview shows the current (unsaved) text', async () => {
    vi.mocked(call).mockImplementation(async (cmd: string) => {
      if (cmd === 'list_hosts') return { ok: true, value: hosts };
      if (cmd === 'fs_read') return { ok: true, value: { size: 20, hash: 'h1', mtime: 1, truncated: false, text: '# Title\n\nSome **bold** text' } };
      return { ok: true, value: null };
    });
    setEditorTarget({ hostId: 'host-1', path: '/home/u/README.md' });
    mount();
    const prev = await screen.findByTestId('editor-md-preview');
    await waitFor(() => expect(prev.querySelector('h1')?.textContent).toBe('Title'));
    expect(prev.querySelector('strong')?.textContent).toBe('bold');
    expect(document.querySelector('.editor-toolbar')).toBeNull();
    const toggle = screen.getByTestId('editor-preview-toggle');
    expect(toggle.textContent).toBe('Edit');
    toggle.click();
    await waitFor(() => expect(screen.queryByTestId('editor-md-preview')).toBeNull());
    const { EditorView } = await import('@codemirror/view');
    const view = EditorView.findFromDOM(document.querySelector('.cm-editor') as HTMLElement)!;
    view.dispatch({ changes: { from: view.state.doc.length, insert: '\n\n## Added' } });
    screen.getByTestId('editor-preview-toggle').click();
    const again = await screen.findByTestId('editor-md-preview');
    await waitFor(() => expect(again.querySelector('h2')?.textContent).toBe('Added'));
  });

  it('other text files have no preview toggle', async () => {
    mount();
    await waitFor(() => expect(document.querySelector('.cm-content')).toBeTruthy());
    expect(screen.queryByTestId('editor-preview-toggle')).toBeNull();
  });
});
