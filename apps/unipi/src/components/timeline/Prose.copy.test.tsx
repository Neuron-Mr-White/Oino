// @vitest-environment jsdom
/** Prose: copying rendered assistant text yields the original markdown (lib/markdown/copyAsMarkdown.ts). */
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { Prose } from './Prose';

const SOURCE = '## Plan\n\n* item **one**\n* item two\n\n```sh\nnpm test\n```';

function copy(): { plain?: string; html?: string } {
  const store = new Map<string, string>();
  const ev = new Event('copy', { bubbles: true, cancelable: true });
  Object.defineProperty(ev, 'clipboardData', { value: { setData: (t: string, v: string) => store.set(t, v) } });
  document.body.dispatchEvent(ev);
  return { plain: store.get('text/plain'), html: store.get('text/html') };
}

function select(a: Node, ao: number, b: Node, bo: number): void {
  const r = document.createRange();
  r.setStart(a, ao);
  r.setEnd(b, bo);
  const s = window.getSelection()!;
  s.removeAllRanges();
  s.addRange(r);
}

afterEach(() => {
  window.getSelection()?.removeAllRanges();
  cleanup();
});

describe('Prose copy', () => {
  it('whole-message selection across the rendered blocks copies the exact source', () => {
    const { container } = render(() => <Prose text={SOURCE} />);
    const root = container.querySelector('.tl-prose')!;
    select(root, 0, root, root.childNodes.length);
    expect(copy().plain).toBe(SOURCE);
  });

  it('a partial selection copies converted markdown plus HTML', () => {
    const { container } = render(() => <Prose text={SOURCE} />);
    const li = container.querySelector('li')!;
    const code = container.querySelector('pre code')!;
    select(li.firstChild!, 0, code.firstChild!, 3);
    const out = copy();
    expect(out.plain).toBe('- item **one**\n- item two\n\n```sh\nnpm\n```');
    expect(out.html).toContain('<strong>one</strong>');
  });
});
