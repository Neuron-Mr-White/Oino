// @vitest-environment jsdom
import { cleanup, render, screen } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { afterEach, describe, expect, it } from 'vitest';
import { TurnEnd } from './Transcript';
import { ReviewViewer, closeReviewViewer } from './ReviewViewer';
import type { TurnFile, TurnFileEdit } from '../../lib/chat/model';
import { getEditorTarget, clearEditorTarget } from '../../lib/editorTarget';
import type { Chat } from '../../lib/chat/store';

afterEach(() => {
  cleanup();
  clearEditorTarget();
  closeReviewViewer();
  // Open-in-editor navigates to #/files/edit; reset so the next test's
  // HashRouter starts at '/'.
  window.location.hash = '#/';
});

function fakeChat(hostId: string): Chat {
  return { hostId } as unknown as Chat;
}

function fileEdit(overrides: Partial<TurnFileEdit> = {}): TurnFileEdit {
  return {
    add: 2,
    del: 1,
    oldText: 'old',
    newText: 'line1\nnew',
    tool: { id: '1', name: 'edit', args: {}, state: 'ok' },
    ...overrides,
  };
}

function file(overrides: Partial<TurnFile> = {}): TurnFile {
  const edits = overrides.edits ?? [fileEdit()];
  const add = overrides.add ?? edits.reduce((n, e) => n + e.add, 0);
  const del = overrides.del ?? edits.reduce((n, e) => n + e.del, 0);
  return {
    path: '/home/u/proj/a.txt',
    add,
    del,
    tool: { id: '1', name: 'edit', args: { edits: [{ oldText: edits[0]!.oldText, newText: edits[0]!.newText }] }, state: 'ok' },
    edits,
    ...overrides,
  };
}

describe('"Changes this turn" -> Open in editor (UNI-155)', () => {
  it('does not show the open-in-editor button without a chat (no host to target)', () => {
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[file()]} />} />
      </HashRouter>
    ));
    expect(screen.queryByLabelText(/Open .* in editor/)).toBeNull();
  });

  it('resolves a relative tool-arg path against the session cwd (UNI-178)', () => {
    const chat = { hostId: 'host-1', state: { session: { cwd: '/home/u/proj' } } } as unknown as Chat;
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[file({ path: 'src/a.txt' })]} chat={chat} />} />
      </HashRouter>
    ));
    screen.getByLabelText(/Open .* in editor/).click();
    expect(getEditorTarget()?.path).toBe('/home/u/proj/src/a.txt');
  });

  it('passes a ~ path through for the host to expand (UNI-178)', () => {
    const chat = { hostId: 'host-1', state: { session: { cwd: '/home/u/proj' } } } as unknown as Chat;
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[file({ path: '~/notes.md' })]} chat={chat} />} />
      </HashRouter>
    ));
    screen.getByLabelText(/Open .* in editor/).click();
    expect(getEditorTarget()?.path).toBe('~/notes.md');
  });

  it('clips at ~2.5 rows with a Show-all drawer when a turn touched many files (UNI-177)', () => {
    const chat = fakeChat('host-1');
    const files = [1, 2, 3, 4, 5].map((n) => file({ path: `/p/f${n}.ts`, edits: [fileEdit({ add: n, del: 1 })] }));
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={files} chat={chat} />} />
      </HashRouter>
    ));
    // Header: colored totals classes present + Show all control.
    const head = document.querySelector('.changes-head')!;
    expect(head.querySelector('.tool-stat .add')!.textContent).toBe('+15');
    expect(head.querySelector('.tool-stat .del')!.textContent).toBe('−5');
    expect(screen.getByTestId('changes-show-all')).toBeTruthy();
    // Collapsed list is clipped.
    expect(document.querySelector('.changes-list')!.classList.contains('clipped')).toBe(true);
    // Show all opens the drawer listing every file.
    screen.getByTestId('changes-show-all').click();
    const sheet = screen.getByTestId('changes-sheet');
    expect(sheet.querySelectorAll('.changes-file').length).toBe(5);
  });

  it('a file tapped in the clipped list opens the drawer with it expanded (UNI-183)', async () => {
    const chat = fakeChat('host-1');
    const files = [1, 2, 3, 4, 5].map((n) => file({ path: `/p/f${n}.ts`, edits: [fileEdit({ add: n, del: 1 })] }));
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={files} chat={chat} />} />
      </HashRouter>
    ));
    const row = document.querySelectorAll<HTMLButtonElement>('.changes-list .changes-file-row-main')[1]!;
    row.click();
    // Nothing expands inside the clipped list…
    expect(document.querySelector('.changes-list .tool-diff')).toBeNull();
    // …the drawer opens with that file's diff.
    const sheet = await screen.findByTestId('changes-sheet');
    const expanded = [...sheet.querySelectorAll<HTMLElement>('.changes-file')].find((el) => el.querySelector('.tool-diff'));
    expect(expanded?.dataset.path).toBe('/p/f2.ts');
  });

  it('no Show-all control for 3 or fewer files', () => {
    const chat = fakeChat('host-1');
    const files = [1, 2, 3].map((n) => file({ path: `/p/f${n}.ts` }));
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={files} chat={chat} />} />
      </HashRouter>
    ));
    expect(screen.queryByTestId('changes-show-all')).toBeNull();
    expect(document.querySelector('.changes-list')!.classList.contains('clipped')).toBe(false);
  });

  it('sets the editor target and navigates when tapped', () => {
    const chat = fakeChat('host-1');
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[file()]} chat={chat} />} />
      </HashRouter>
    ));
    const button = screen.getByLabelText(/Open .* in editor/);
    button.click();
    const target = getEditorTarget();
    expect(target?.hostId).toBe('host-1');
    expect(target?.path).toBe('/home/u/proj/a.txt');
    expect(target?.line).toBeGreaterThanOrEqual(1);
  });

  it('Open from the Show-all drawer closes the drawer and navigates (UNI-192)', async () => {
    const chat = fakeChat('host-1');
    const files = [1, 2, 3, 4, 5].map((n) => file({ path: `/p/f${n}.ts`, edits: [fileEdit({ add: n, del: 1 })] }));
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={files} chat={chat} />} />
        <Route path="/files/edit" component={() => <div data-testid="editor-route" />} />
      </HashRouter>
    ));
    screen.getByTestId('changes-show-all').click();
    const sheet = await screen.findByTestId('changes-sheet');
    const openBtn = sheet.querySelectorAll<HTMLButtonElement>('.changes-file-open')[1]!;
    openBtn.click();
    expect(getEditorTarget()?.path).toBe('/p/f2.ts');
    // The route actually changed — the bug was the drawer's portal staying
    // on top and swallowing the tap / hiding the editor underneath.
    expect(await screen.findByTestId('editor-route')).toBeTruthy();
    // The drawer is a Corvu portal that would otherwise sit over the
    // editor route: it must have closed.
    expect(screen.queryByTestId('changes-sheet')).toBeNull();
  });
});

describe('"Changes this turn" -> per-edit rows (UNI-192)', () => {
  it('a single-edit file behaves as one row, as before', () => {
    const chat = fakeChat('host-1');
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[file()]} chat={chat} />} />
      </HashRouter>
    ));
    expect(document.querySelectorAll('.changes-edit').length).toBe(0);
    const row = document.querySelector<HTMLButtonElement>('.changes-file-row-main')!;
    row.click();
    expect(document.querySelector('.changes-file .tool-diff')).not.toBeNull();
  });

  it('a file with several edits lists each as its own sub-row, expanding independently', () => {
    const chat = fakeChat('host-1');
    const f = file({
      path: '/p/multi.ts',
      edits: [fileEdit({ oldText: 'a', newText: 'A', add: 1, del: 1 }), fileEdit({ oldText: 'b', newText: 'B\nB2', add: 2, del: 1 })],
    });
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[f]} chat={chat} />} />
      </HashRouter>
    ));
    const rows = document.querySelectorAll<HTMLButtonElement>('.changes-edit-row-main');
    expect(rows.length).toBe(2);
    expect(rows[0]!.textContent).toMatch(/edit 1/);
    expect(rows[1]!.textContent).toMatch(/edit 2/);
    // Expanding the second edit shows only its diff.
    rows[1]!.click();
    const diffs = document.querySelectorAll('.changes-file .tool-diff');
    expect(diffs.length).toBe(1);
    expect(diffs[0]!.textContent).toMatch(/B2/);
    // The first edit's row is still collapsed.
    expect(rows[0]!.getAttribute('aria-expanded')).toBe('false');
  });
});

describe('"Changes this turn" -> clips by rendered rows, not file count (UNI-297)', () => {
  it('2 files with 6 and 4 edits clip and show Show-all, even though files.length is only 2', () => {
    const chat = fakeChat('host-1');
    const files = [
      file({ path: '/p/styles.css', edits: Array.from({ length: 6 }, (_, i) => fileEdit({ oldText: `a${i}`, newText: `A${i}` })) }),
      file({ path: '/p/main.js', edits: Array.from({ length: 4 }, (_, i) => fileEdit({ oldText: `b${i}`, newText: `B${i}` })) }),
    ];
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={files} chat={chat} />} />
      </HashRouter>
    ));
    expect(screen.getByTestId('changes-show-all')).toBeTruthy();
    expect(document.querySelector('.changes-list')!.classList.contains('clipped')).toBe(true);
    // Show all still lists every file/edit in the drawer.
    screen.getByTestId('changes-show-all').click();
    const sheet = screen.getByTestId('changes-sheet');
    expect(sheet.querySelectorAll('.changes-file').length).toBe(2);
    expect(sheet.querySelectorAll('.changes-edit').length).toBe(10);
  });

  it('1 file with 1 edit does not clip (3 rows or fewer)', () => {
    const chat = fakeChat('host-1');
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[file()]} chat={chat} />} />
      </HashRouter>
    ));
    expect(screen.queryByTestId('changes-show-all')).toBeNull();
    expect(document.querySelector('.changes-list')!.classList.contains('clipped')).toBe(false);
  });

  it('1 file with many edits also clips (rows, not file count, drive it)', () => {
    const chat = fakeChat('host-1');
    const f = file({ path: '/p/many.ts', edits: Array.from({ length: 5 }, (_, i) => fileEdit({ oldText: `a${i}`, newText: `A${i}` })) });
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[f]} chat={chat} />} />
      </HashRouter>
    ));
    // 1 file head row + 5 edit rows = 6 rows > 3.
    expect(screen.getByTestId('changes-show-all')).toBeTruthy();
    expect(document.querySelector('.changes-list')!.classList.contains('clipped')).toBe(true);
  });
});

describe('"Changes this turn" -> edit row line label is a real file line or omitted (UNI-297)', () => {
  it('a multi-edit file shows each edit\'s real line when known', () => {
    const chat = fakeChat('host-1');
    const f = file({
      path: '/p/multi.ts',
      edits: [fileEdit({ oldText: 'a', newText: 'A', realLine: 10 }), fileEdit({ oldText: 'b', newText: 'B', realLine: undefined })],
    });
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[f]} chat={chat} />} />
      </HashRouter>
    ));
    const rows = document.querySelectorAll<HTMLButtonElement>('.changes-edit-row-main');
    expect(rows[0]!.textContent).toMatch(/edit 1.*L10/s);
    // No real line known for edit 2: no "L…" shown at all (not a wrong guess).
    expect(rows[1]!.textContent).toMatch(/edit 2/);
    expect(rows[1]!.textContent).not.toMatch(/L/);
  });

  it('Open in editor uses the real line when known, else line 1', () => {
    const chat = fakeChat('host-1');
    const f = file({ path: '/p/real.ts', edits: [fileEdit({ oldText: 'a', newText: 'A', realLine: 77 })] });
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route path="/" component={() => <TurnEnd files={[f]} chat={chat} />} />
      </HashRouter>
    ));
    screen.getByLabelText(/Open .* in editor/).click();
    expect(getEditorTarget()?.line).toBe(77);
  });
});

describe('"Changes this turn" -> Review (UNI-192)', () => {
  it('opens the full-screen viewer at the tapped edit', () => {
    const chat = fakeChat('host-1');
    const f = file({
      path: '/p/multi.ts',
      edits: [fileEdit({ oldText: 'a', newText: 'A' }), fileEdit({ oldText: 'b', newText: 'B' })],
    });
    render(() => (
      <HashRouter root={(p) => p.children}>
        <Route
          path="/"
          component={() => (
            <>
              <TurnEnd files={[f]} chat={chat} />
              <ReviewViewer />
            </>
          )}
        />
      </HashRouter>
    ));
    const reviewBtns = document.querySelectorAll<HTMLButtonElement>('.changes-edit-review');
    expect(reviewBtns.length).toBe(2);
    reviewBtns[1]!.click();
    const viewer = screen.getByTestId('review-viewer');
    expect(viewer.textContent).toMatch(/edit 2/);
  });
});
