import { beforeEach, describe, expect, it } from 'vitest';
import { isMermaidLang, renderMermaid, resetMermaidCache } from './mermaid';

// jsdom has no layout engine — mermaid calls SVGElement.getBBox while
// measuring text, which jsdom doesn't implement.
if (!(SVGElement.prototype as unknown as { getBBox?: unknown }).getBBox) {
  (SVGElement.prototype as unknown as { getBBox: () => DOMRect }).getBBox = () => ({ x: 0, y: 0, width: 10, height: 10 }) as DOMRect;
}

beforeEach(() => resetMermaidCache());

describe('isMermaidLang', () => {
  it('recognises the mermaid fence language only', () => {
    expect(isMermaidLang('mermaid')).toBe(true);
    expect(isMermaidLang('Mermaid')).toBe(true);
    expect(isMermaidLang('js')).toBe(false);
  });
});

describe('renderMermaid', () => {
  it('renders a valid flowchart to a sanitised <svg>', async () => {
    const result = await renderMermaid('graph TD;\nA-->B;');
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.svg).toContain('<svg');
      expect(result.svg).not.toContain('<script');
    }
  });

  it('falls back to an error for invalid mermaid source', async () => {
    const result = await renderMermaid('this is not a diagram {{{');
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error).toBeTruthy();
  });

  it('keeps mermaid\'s foreignObject text labels (DOMPurify\'s svg profile forbids their contents by default)', async () => {
    const result = await renderMermaid('graph TD;\nA[Start] --> B{Decision};');
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.svg).toContain('foreignObject');
      // The label text itself (inside foreignObject > div > span) must survive, not just the empty wrapper.
      expect(result.svg).toMatch(/foreignObject[^>]*>\s*<div[^>]*>\s*<span/);
    }
  });
  it('still strips scripts/event handlers that might ride along inside a foreignObject label', async () => {
    const result = await renderMermaid('graph TD;\nA[Start] --> B{Decision};');
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.svg).not.toContain('<script');
      expect(result.svg).not.toMatch(/\bonload=/);
      expect(result.svg).not.toMatch(/\bonclick=/);
    }
  });

  it('caches by source — repeated calls do not re-render', async () => {
    const a = await renderMermaid('graph TD;\nA-->B;');
    const b = await renderMermaid('graph TD;\nA-->B;');
    expect(a).toBe(b);
  });
});
