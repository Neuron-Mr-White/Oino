import { describe, expect, it } from 'vitest';
import { renderBlocks, renderChatMarkdown, splitBlocks } from './markdown';

describe('chat markdown', () => {
  it('code blocks get a language header with Copy, html is escaped', () => {
    const html = renderChatMarkdown('```ts\nconst a = "<b>";\n```');
    expect(html).toContain('class="cb-lang">ts<');
    expect(html).toContain('data-copy');
    expect(html).toContain('&lt;b&gt;');
  });
  it('drops scripts and javascript: links', () => {
    const html = renderChatMarkdown('<script>alert(1)</script>\n\n[x](javascript:alert(1)) [ok](https://a.b)');
    expect(html).not.toContain('<script');
    expect(html).not.toMatch(/href="javascript/);
    expect(html).toContain('href="https://a.b"');
  });
  it('renders GFM task-list checkboxes (disabled, checked state preserved)', () => {
    const html = renderChatMarkdown('- [x] done\n- [ ] todo');
    expect(html).toContain('<input');
    expect(html).toMatch(/type="checkbox"/);
    expect(html).toContain('disabled');
  });
  it('strips a non-checkbox input even if raw HTML tries to smuggle one in', () => {
    const html = renderChatMarkdown('<input type="text" value="hack">');
    expect(html).not.toContain('<input');
  });
  it('long code blocks (>20 lines) get line numbers and a wrap toggle; short ones do not', () => {
    const long = Array.from({ length: 25 }, (_, i) => `line ${i}`).join('\n');
    const html = renderChatMarkdown('```\n' + long + '\n```');
    expect(html).toContain('line-numbers');
    expect(html).toContain('data-wrap-toggle');
    expect(html).toContain('cb-line');
    const short = renderChatMarkdown('```\na\nb\nc\n```');
    expect(short).not.toContain('line-numbers');
    expect(short).not.toContain('data-wrap-toggle');
  });
  it('splits into blocks, keeping fences and lists whole', () => {
    expect(splitBlocks('a\n\n```\nx\n\ny\n```\n\n- one\n\n- two\n\nend')).toEqual(['a\n', '```\nx\n\ny\n```', '- one\n\n- two\n', 'end']);
    expect(splitBlocks('text\n\n```py\nopen fence')).toEqual(['text\n', '```py\nopen fence']);
  });
  it('renderBlocks keeps one html per block', () => {
    expect(renderBlocks('# H\n\npara', true)).toHaveLength(2);
  });
});

describe('streamed blocks render like the whole message (UNI-141)', () => {
  const cases = [
    'Steps:\n\n1. **Phone:** install it.\n2. **PC:**\n   ```\n   cd ~/x\n   ./run.sh\n   ```\n   `install` copies it, `start` runs it.\n\nDone.',
    '- **Install:**\n  ```\n  curl -fsSL https://x/install.sh | sh\n  ```\n  It installs the host.\n- Next item\n\nAfter.',
    '## Title\n- **Size:** 34 px.\n\n- **Mapping:** same order.\n\nEnd.',
    'Intro\n## Heading\n| a | b |\n|---|---|\n| 1 | 2 |\n\nTail',
  ];
  const norm = (h: string) => h.replace(/\s+/g, ' ').replace(/> </g, '><').trim();
  for (const text of cases) {
    it(text.slice(0, 30).replace(/\n/g, '⏎'), () => {
      for (let i = 1; i < text.length; i += 3) renderBlocks(text.slice(0, i), true);
      expect(norm(renderBlocks(text, false).join(''))).toBe(norm(renderChatMarkdown(text)));
    });
  }
  it('a single newline is a line break, as in the TUI', () => {
    expect(renderChatMarkdown('line one\nline two')).toContain('<br>');
  });
});
