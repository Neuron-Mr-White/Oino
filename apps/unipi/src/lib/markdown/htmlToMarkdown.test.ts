// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { renderMarkdown } from './core';
import { htmlToMarkdown } from './htmlToMarkdown';

function dom(html: string): HTMLElement {
  const el = document.createElement('div');
  el.innerHTML = html;
  return el;
}

/** source → rendered DOM → markdown. */
const roundTrip = (src: string, images = false): string => htmlToMarkdown(dom(renderMarkdown(src, { allowImages: images })));

describe('htmlToMarkdown — round trip through renderMarkdown', () => {
  it('inline emphasis, strong, strikethrough and code', () => {
    expect(roundTrip('a **bold** and _em_ and ~~gone~~ and `code`')).toBe('a **bold** and _em_ and ~~gone~~ and `code`');
    expect(roundTrip('use ``a ` b`` here')).toBe('use ``a ` b`` here');
  });

  it('headings h1–h6', () => {
    const src = ['# One', '## Two', '### Three', '#### Four', '##### Five', '###### Six'].join('\n\n');
    expect(roundTrip(src)).toBe(src);
  });

  it('fenced code with a language, without one, and containing backticks', () => {
    expect(roundTrip('```ts\nconst a = 1;\n\nconst b = 2;\n```')).toBe('```ts\nconst a = 1;\n\nconst b = 2;\n```');
    expect(roundTrip('```\nplain\n```')).toBe('```\nplain\n```');
    expect(roundTrip('````md\n```js\nx\n```\n````')).toBe('````md\n```js\nx\n```\n````');
  });

  it('unordered, ordered and nested lists', () => {
    const src = '- one\n- two\n  - two a\n  - two b\n    1. deep\n    2. deeper\n- three';
    expect(roundTrip(src)).toBe(src);
    expect(roundTrip('1. a\n2. b\n3. c')).toBe('1. a\n2. b\n3. c');
    expect(roundTrip('3. c\n4. d')).toBe('3. c\n4. d');
  });

  it('loose lists keep their blank lines', () => {
    expect(roundTrip('- a\n\n- b')).toBe('- a\n\n- b');
  });

  it('task lists', () => {
    expect(roundTrip('- [x] done\n- [ ] todo')).toBe('- [x] done\n- [ ] todo');
  });

  it('a fenced block inside a list item stays indented under it', () => {
    expect(roundTrip('- run:\n  ```sh\n  npm test\n  ```\n- next')).toBe('- run:\n  ```sh\n  npm test\n  ```\n- next');
  });

  it('links, with and without a title, and bare urls', () => {
    expect(roundTrip('see [docs](https://x.dev/a) now')).toBe('see [docs](https://x.dev/a) now');
    expect(roundTrip('[t](https://x.dev "Title")')).toBe('[t](https://x.dev "Title")');
    expect(roundTrip('go to https://x.dev/a')).toBe('go to https://x.dev/a');
  });

  it('images (when the renderer allows them)', () => {
    expect(roundTrip('![alt text](https://x.dev/i.png "cap")', true)).toBe('![alt text](https://x.dev/i.png "cap")');
  });

  it('blockquotes, nested and multi-paragraph', () => {
    expect(roundTrip('> quoted\n>\n> second')).toBe('> quoted\n>\n> second');
    expect(roundTrip('> a\n>\n> > nested')).toBe('> a\n>\n> > nested');
  });

  it('tables with alignment and inline formatting', () => {
    const src = '| Name | Qty | Note |\n| :--- | ---: | :---: |\n| **a** | 1 | `x` |\n| b | 22 | [l](https://x.dev) |';
    expect(roundTrip(src)).toBe(src);
  });

  it('an escaped pipe inside a cell survives', () => {
    expect(roundTrip('| a |\n| --- |\n| x \\| y |')).toBe('| a |\n| --- |\n| x \\| y |');
  });

  it('horizontal rules and paragraph breaks', () => {
    expect(roundTrip('one\n\n---\n\ntwo')).toBe('one\n\n---\n\ntwo');
  });

  it('literal markdown characters in prose are escaped, intraword underscores are not', () => {
    expect(roundTrip('snake_case and \\*not em\\* and 5 * 3')).toBe('snake_case and \\*not em\\* and 5 * 3');
  });

  it('mermaid fences come back from the source attribute', () => {
    expect(roundTrip('```mermaid\ngraph TD; A-->B\n```')).toBe('```mermaid\ngraph TD; A-->B\n```');
  });

  it('diff fences come back from the raw copy', () => {
    const src = '```diff\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n```';
    expect(roundTrip(src)).toBe(src);
  });

  it('a long code block (line-numbered spans) converts to its raw lines', () => {
    const body = Array.from({ length: 25 }, (_, i) => `line ${i + 1}`).join('\n');
    expect(roundTrip('```txt\n' + body + '\n```')).toBe('```txt\n' + body + '\n```');
  });
});

describe('htmlToMarkdown — fragments and chrome', () => {
  it('drops code-block chrome (header, Copy / Wrap buttons)', () => {
    const out = htmlToMarkdown(dom(renderMarkdown('```js\nx()\n```')));
    expect(out).not.toMatch(/Copy|Wrap/);
  });

  it('drops path-copy buttons, the Stopped badge and KaTeX MathML', () => {
    const el = dom('<p>see <code>a/b.ts</code><button class="code-copy" data-copy-path></button> <span class="katex"><span class="katex-mathml">ZZ</span><span class="katex-html">x²</span></span></p><span class="chat-stopped">Stopped</span>');
    expect(htmlToMarkdown(el)).toBe('see `a/b.ts` x²');
  });

  it('prefers the original TeX from a data attribute', () => {
    expect(htmlToMarkdown(dom('<p>so <span data-tex="x^2">x²</span> ok</p>'))).toBe('so $x^2$ ok');
    expect(htmlToMarkdown(dom('<div><span data-tex="\\sum_i i" data-display>Σ</span></div>'))).toBe('$$\\sum_i i$$');
  });

  it('orphan <li>s (as a Range clone yields) become list items, and orphan rows a table', () => {
    expect(htmlToMarkdown(dom('<li>a</li><li>b</li>'))).toBe('- a\n- b');
    const frag = document.createDocumentFragment();
    frag.appendChild(dom('<table><tbody><tr><td>1</td><td>2</td></tr></tbody></table>').querySelector('tr')!);
    expect(htmlToMarkdown(frag)).toBe('| 1 | 2 |\n| --- | --- |');
  });

  it('a <br> becomes a hard line break', () => {
    expect(htmlToMarkdown(dom('<p>a<br>b</p>'))).toBe('a  \nb');
  });

  it('whitespace-only emphasis is not wrapped; edge whitespace moves outside the markers', () => {
    expect(htmlToMarkdown(dom('<p>x<strong> </strong>y</p>'))).toBe('x y');
    expect(htmlToMarkdown(dom('<p>x<strong> bold </strong>y</p>'))).toBe('x **bold** y');
  });
});
