import { afterEach, describe, expect, it } from 'vitest';
import { attPath, hydrateAttachments, plainText, renderMarkdown, setAttachmentBase } from './markdown';
import { namedFile, insertAtCursor } from './attach';
import { resetFileUrlCache } from './api';
import { setBoardTransport } from './transport';
import { fakeBoard } from './testing';

afterEach(() => {
  setAttachmentBase(null);
  setBoardTransport(undefined);
  resetFileUrlCache();
});

describe('board markdown', () => {
  it('sanitises and renders GFM', () => {
    const html = renderMarkdown('**bold** <script>alert(1)</script>\n- item');
    expect(html).toContain('<strong>bold</strong>');
    expect(html).not.toContain('<script');
    expect(html).toContain('<li>item</li>');
  });

  it('rewrites att: refs to daemon paths (resolved later through the transport)', () => {
    setAttachmentBase('proj-1');
    expect(attPath('att:T-4/abc12345-shot.png')).toBe('/api/files/proj-1/T-4/abc12345-shot.png');
    const html = renderMarkdown('![shot](att:T-4/abc12345-shot.png) and [log](att:T-4/abc12345-run.log)');
    expect(html).toContain('data-att-path="/api/files/proj-1/T-4/abc12345-shot.png"');
    expect(html).toContain('class="att-chip att-text"');
    expect(html).not.toContain('att:T-4');
  });

  it('hydrates attachment nodes with object URLs', async () => {
    setAttachmentBase('proj-1');
    const fake = fakeBoard({ routes: { 'GET /api/files/proj-1/T-4/abc12345-shot.png': () => ({ text: 'PNG' }) } });
    setBoardTransport(fake.transport);
    const created: string[] = [];
    URL.createObjectURL = ((blob: Blob) => {
      created.push(blob.type);
      return 'blob:fake/1';
    }) as typeof URL.createObjectURL;
    const root = document.createElement('div');
    root.innerHTML = renderMarkdown('![shot](att:T-4/abc12345-shot.png)');
    await hydrateAttachments(root, 'pc');
    expect(root.querySelector('img')!.getAttribute('src')).toBe('blob:fake/1');
    expect(fake.requests[0]!.hostId).toBe('pc');
  });

  it('plainText flattens markdown for card callouts', () => {
    expect(plainText('Need **a decision**: pin `0.95`?')).toBe('Need a decision: pin 0.95?');
  });

  it('names pasted images and inserts snippets on their own line', () => {
    const named = namedFile(new File(['x'], 'image.png', { type: 'image/png' }), new Date('2026-10-07T14:02:00Z'));
    expect(named.name).toBe('screenshot-2026-10-07-1402.png');
    expect(insertAtCursor(undefined, 'body', '![x](att:A/b.png)')).toBe('body\n![x](att:A/b.png)');
  });
});
