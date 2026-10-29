import { afterEach, describe, expect, it } from 'vitest';
import { adjacentIndex, choiceKey, composerKey, dragHasFiles, filesKey, inTextField, nextFontSize, notifyPlan, notifyText, shortcuts, splitDropped, termKey, windowTitle, type NotifyItem } from './desktop';
import { setPlatformOverride } from './platform';

const k = (key: string, m: Partial<{ ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean; isComposing: boolean }> = {}) => ({ key, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...m });

afterEach(() => setPlatformOverride(undefined));

describe('composerKey', () => {
  const idle = { running: false, empty: false };
  const busy = { running: true, empty: false };
  it('Enter sends, Shift+Enter is a new line', () => {
    expect(composerKey(k('Enter'), idle)).toBe('send');
    expect(composerKey(k('Enter', { shiftKey: true }), idle)).toBeNull();
    expect(composerKey(k('Enter'), { running: false, empty: true })).toBeNull();
  });
  it('while pi works: Enter = last choice, Ctrl/⌘+Enter = choose', () => {
    expect(composerKey(k('Enter'), busy)).toBe('send-default');
    expect(composerKey(k('Enter', { ctrlKey: true }), busy)).toBe('choose');
    expect(composerKey(k('Enter', { metaKey: true }), busy)).toBe('choose');
    expect(composerKey(k('Enter', { ctrlKey: true }), idle)).toBe('send');
  });
  it('Esc stops a run only', () => {
    expect(composerKey(k('Escape'), busy)).toBe('stop');
    expect(composerKey(k('Escape'), idle)).toBeNull();
  });
  it('IME composition is never a send', () => {
    expect(composerKey(k('Enter', { isComposing: true }), idle)).toBeNull();
  });
});

describe('choiceKey', () => {
  it('S/Q pick, arrows switch, Enter picks highlighted, Esc closes', () => {
    expect(choiceKey(k('s'), 'after')).toEqual({ pick: 'steer' });
    expect(choiceKey(k('Q'), 'steer')).toEqual({ pick: 'after' });
    expect(choiceKey(k('ArrowRight'), 'steer')).toEqual({ highlight: 'after' });
    expect(choiceKey(k('Enter'), 'after')).toEqual({ pick: 'after' });
    expect(choiceKey(k('Escape'), 'steer')).toEqual({ close: true });
    expect(choiceKey(k('x'), 'steer')).toBeNull();
    expect(choiceKey(k('s', { ctrlKey: true }), 'steer')).toBeNull();
  });
});

describe('adjacentIndex', () => {
  it('wraps both ways; none open starts at an end', () => {
    expect(adjacentIndex(3, 0, -1)).toBe(2);
    expect(adjacentIndex(3, 2, 1)).toBe(0);
    expect(adjacentIndex(3, -1, 1)).toBe(0);
    expect(adjacentIndex(3, -1, -1)).toBe(2);
    expect(adjacentIndex(0, 0, 1)).toBeUndefined();
  });
});

describe('drag and drop', () => {
  it('splits images from other files (svg is a file)', () => {
    const png = new File(['x'], 'a.png', { type: 'image/png' });
    const svg = new File(['x'], 'b.svg', { type: 'image/svg+xml' });
    const txt = new File(['x'], 'c.txt', { type: 'text/plain' });
    expect(splitDropped([png, svg, txt])).toEqual({ images: [png], others: [svg, txt] });
  });
  it('only OS file drags count', () => {
    expect(dragHasFiles({ types: ['Files'] } as unknown as DataTransfer)).toBe(true);
    expect(dragHasFiles({ types: ['text/plain'] } as unknown as DataTransfer)).toBe(false);
    expect(dragHasFiles(null)).toBe(false);
  });
});

describe('termKey', () => {
  it('Ctrl+Shift+C/V copy/paste; ⌘C/⌘V on a Mac', () => {
    expect(termKey(k('C', { ctrlKey: true, shiftKey: true }))).toEqual({ kind: 'copy' });
    expect(termKey(k('V', { ctrlKey: true, shiftKey: true }))).toEqual({ kind: 'paste' });
    expect(termKey(k('c', { metaKey: true }), true)).toEqual({ kind: 'copy' });
    expect(termKey(k('v', { metaKey: true }), true)).toEqual({ kind: 'paste' });
  });
  it('plain Ctrl+C / Ctrl+K stay the shell\'s; Ctrl+digit and Ctrl+Shift+K are the app\'s', () => {
    expect(termKey(k('c', { ctrlKey: true }))).toBeNull();
    expect(termKey(k('k', { ctrlKey: true }))).toBeNull();
    expect(termKey(k('2', { ctrlKey: true }))).toEqual({ kind: 'app' });
    expect(termKey(k('K', { ctrlKey: true, shiftKey: true }))).toEqual({ kind: 'app' });
    expect(termKey(k('w', { metaKey: true }), true)).toEqual({ kind: 'app' });
  });
  it('font size; Alt goes to xterm', () => {
    expect(termKey(k('=', { ctrlKey: true }))).toEqual({ kind: 'font', delta: 1 });
    expect(termKey(k('+', { ctrlKey: true, shiftKey: true }))).toEqual({ kind: 'font', delta: 1 });
    expect(termKey(k('-', { ctrlKey: true }))).toEqual({ kind: 'font', delta: -1 });
    expect(termKey(k('0', { ctrlKey: true }))).toEqual({ kind: 'font', delta: 0 });
    expect(termKey(k('b', { altKey: true }))).toBeNull();
  });
  it('plain keys go the IME path', () => {
    expect(termKey(k('a'))).toBeNull();
    expect(termKey(k('Enter'))).toBeNull();
  });
  it('nextFontSize clamps and resets', () => {
    expect(nextFontSize(14, 1)).toBe(15);
    expect(nextFontSize(32, 1)).toBe(32);
    expect(nextFontSize(8, -1)).toBe(8);
    expect(nextFontSize(20, 0)).toBe(14);
  });
});

describe('filesKey', () => {
  it('moves, opens, goes up, renames, deletes', () => {
    expect(filesKey(k('ArrowDown'), -1, 3)).toEqual({ kind: 'move', to: 0 });
    expect(filesKey(k('ArrowDown'), 2, 3)).toEqual({ kind: 'move', to: 2 });
    expect(filesKey(k('ArrowUp'), 0, 3)).toEqual({ kind: 'move', to: 0 });
    expect(filesKey(k('End'), 0, 3)).toEqual({ kind: 'move', to: 2 });
    expect(filesKey(k('Enter'), 1, 3)).toEqual({ kind: 'open' });
    expect(filesKey(k('Enter'), -1, 3)).toBeNull();
    expect(filesKey(k('Backspace'), -1, 0)).toEqual({ kind: 'up' });
    expect(filesKey(k('F2'), 1, 3)).toEqual({ kind: 'rename' });
    expect(filesKey(k('Delete'), 1, 3)).toEqual({ kind: 'delete' });
    expect(filesKey(k('ArrowDown', { ctrlKey: true }), 0, 3)).toBeNull();
    expect(filesKey(k('ArrowDown'), -1, 0)).toBeNull();
  });
  it('inTextField', () => {
    const input = document.createElement('input');
    const div = document.createElement('div');
    expect(inTextField(input)).toBe(true);
    expect(inTextField(div)).toBe(false);
    expect(inTextField(null)).toBe(false);
  });
});

describe('notifications', () => {
  const item = (id: string, group: NotifyItem['group'] = 'waiting'): NotifyItem => ({ id, group, title: 'math helpers', hostName: 'coffee', kind: 'select' });
  it('first look only seeds; only new unread waiting/finished while unfocused', () => {
    const items = [item('a'), item('b', 'finished'), item('c', 'other'), item('d')];
    expect(notifyPlan(undefined, items, { focused: false, isRead: () => false })).toEqual([]);
    expect(notifyPlan(new Set(['a']), items, { focused: false, isRead: (id) => id === 'd' }).map((i) => i.id)).toEqual(['b']);
    expect(notifyPlan(new Set(), items, { focused: true, isRead: () => false })).toEqual([]);
  });
  it('wording', () => {
    expect(notifyText({ ...item('a'), what: 'Pick a model' })).toEqual({ title: 'math helpers needs you', body: 'Pick a model' });
    expect(notifyText(item('b', 'finished'))).toEqual({ title: 'math helpers finished', body: 'pi on coffee is done.' });
  });
  it('window title', () => {
    expect(windowTitle(['Chat', 'math helpers'])).toBe('Chat — math helpers — UniPi');
    expect(windowTitle([undefined, ' '])).toBe('UniPi');
  });
});

describe('shortcut list', () => {
  it('uses the platform modifier', () => {
    const all = shortcuts('⌘').flatMap((g) => g.items.map((i) => i.keys));
    expect(all).toContain('⌘ K');
    expect(all.some((x) => x.includes('Ctrl'))).toBe(false);
  });
});
