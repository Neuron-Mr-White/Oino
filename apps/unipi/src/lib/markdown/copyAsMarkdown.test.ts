// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { renderMarkdown } from './core';
import { registerMarkdownSource } from './copyAsMarkdown';

const SOURCE = [
  '# Title',
  '',
  'Intro with **bold**, _em_ and `inline`.',
  '',
  '- one',
  '- two',
  '  - nested',
  '',
  '```ts',
  'const a = 1;',
  'const b = 2;',
  '```',
  '',
  '| a | b |',
  '| --- | --- |',
  '| 1 | 2 |',
  '',
  '> quote',
].join('\n');

function mount(source = SOURCE): HTMLElement {
  const root = document.createElement('div');
  root.className = 'chat-prose md';
  root.innerHTML = renderMarkdown(source); // renderMarkdown installs the document listener
  registerMarkdownSource(root, () => source);
  document.body.appendChild(root);
  return root;
}

type Clip = { plain: string | undefined; html: string | undefined; prevented: boolean };

/** Fires a copy event with the current selection and reports what was written. */
function copy(): Clip {
  const store = new Map<string, string>();
  const ev = new Event('copy', { bubbles: true, cancelable: true });
  Object.defineProperty(ev, 'clipboardData', { value: { setData: (t: string, v: string) => store.set(t, v) } });
  document.body.dispatchEvent(ev);
  return { plain: store.get('text/plain'), html: store.get('text/html'), prevented: ev.defaultPrevented };
}

function select(startNode: Node, startOffset: number, endNode: Node, endOffset: number): void {
  const range = document.createRange();
  range.setStart(startNode, startOffset);
  range.setEnd(endNode, endOffset);
  const sel = window.getSelection()!;
  sel.removeAllRanges();
  sel.addRange(range);
}

afterEach(() => {
  window.getSelection()?.removeAllRanges();
  document.body.innerHTML = '';
});

describe('copy as markdown', () => {
  it('a partial selection copies its markdown as text/plain and its HTML as text/html', () => {
    const root = mount();
    const p = root.querySelector('p')!;
    // "Intro with **bold**, _em_" — from the paragraph start through the end of <em>.
    select(p.firstChild!, 6, p.querySelector('em')!.firstChild!, 2);
    const out = copy();
    expect(out.prevented).toBe(true);
    expect(out.plain).toBe('with **bold**, _em_');
    expect(out.html).toContain('<strong>bold</strong>');
    expect(out.html).toContain('<em>em</em>');
  });

  it('a selection across several blocks keeps headings, lists and code fences', () => {
    const root = mount();
    select(root.querySelector('h1')!, 0, root.querySelector('pre code')!.firstChild!, 5);
    const out = copy();
    expect(out.plain).toBe(
      ['# Title', '', 'Intro with **bold**, _em_ and `inline`.', '', '- one', '- two', '  - nested', '', '```ts', 'const', '```'].join('\n'),
    );
    expect(out.html).toContain('<h1>');
    expect(out.html).not.toMatch(/Copy|cb-head|<button/);
  });

  it('selecting list items only copies a list (the dropped <ul> is restored)', () => {
    const root = mount();
    const lis = root.querySelectorAll('ul > li');
    select(lis[0]!.firstChild!, 0, lis[1]!.firstChild!, 3);
    expect(copy().plain).toBe('- one\n- two');
  });

  it('table rows copy as a GFM table', () => {
    const root = mount();
    const cells = root.querySelectorAll('td');
    select(root.querySelector('th')!, 0, cells[1]!, 1);
    expect(copy().plain).toBe('| a | b |\n| --- | --- |\n| 1 | 2 |');
  });

  it('the whole message copies the exact source', () => {
    const root = mount();
    select(root, 0, root, root.childNodes.length);
    const out = copy();
    expect(out.plain).toBe(SOURCE);
    expect(out.html).toContain('<h1>Title</h1>');
    expect(out.html).not.toContain('<button');
  });

  it('a whole message whose source has constructs the converter would rewrite is still exact', () => {
    const src = '* star list\n* second\n\n__strong__ and *em*';
    const root = mount(src);
    select(root, 0, root, root.childNodes.length);
    expect(copy().plain).toBe(src);
  });

  it('a selection inside one code block copies the raw code, without fences', () => {
    const root = mount();
    const code = root.querySelector('pre code')!;
    select(code.firstChild!, 0, code.firstChild!, code.textContent!.length);
    const out = copy();
    expect(out.plain).toBe('const a = 1;\nconst b = 2;');
    expect(out.html).toBe('<pre><code>const a = 1;\nconst b = 2;</code></pre>');
  });

  it('part of a code block copies just that text', () => {
    const root = mount();
    const code = root.querySelector('pre code')!;
    select(code.firstChild!, 6, code.firstChild!, 11);
    expect(copy().plain).toBe('a = 1');
  });

  it('syntax-highlight spans do not leak into the raw code', () => {
    const root = mount('```js\nlet x = 1;\n```');
    const code = root.querySelector('pre code')!;
    code.innerHTML = '<span class="tok-kw">let</span> x = <span class="tok-num">1</span>;';
    select(code, 0, code, code.childNodes.length);
    expect(copy().plain).toBe('let x = 1;');
  });

  it('a selection that leaves the markdown root keeps the default copy', () => {
    const root = mount();
    const outside = document.createElement('p');
    outside.textContent = 'outside';
    document.body.appendChild(outside);
    select(root.querySelector('p')!.firstChild!, 0, outside.firstChild!, 3);
    const out = copy();
    expect(out.prevented).toBe(false);
    expect(out.plain).toBeUndefined();
  });

  it('a collapsed selection and a selection in a textarea keep the default copy', () => {
    const root = mount();
    select(root.querySelector('p')!.firstChild!, 2, root.querySelector('p')!.firstChild!, 2);
    expect(copy().prevented).toBe(false);

    const ta = document.createElement('textarea');
    ta.value = 'hello';
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    expect(copy().prevented).toBe(false);
  });

  it('a root with no registered source converts even for a whole selection', () => {
    const root = document.createElement('div');
    root.className = 'md';
    root.innerHTML = renderMarkdown('hello **world**');
    document.body.appendChild(root);
    select(root, 0, root, root.childNodes.length);
    expect(copy().plain).toBe('hello **world**');
  });

  it('chrome (Copy button, Stopped badge) is not part of the whole-message comparison', () => {
    const root = mount('plain text');
    const stopped = document.createElement('span');
    stopped.className = 'chat-stopped';
    stopped.textContent = 'Stopped';
    root.appendChild(stopped);
    select(root, 0, root, root.childNodes.length);
    expect(copy().plain).toBe('plain text');
  });
});
