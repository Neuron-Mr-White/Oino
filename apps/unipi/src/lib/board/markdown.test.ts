import { afterEach, describe, expect, it } from 'vitest';
import { attPath, hydrateAttachments, plainText, renderMarkdown, setAttachmentBase } from './markdown';
import { namedFile, insertAtCursor } from './attach';
import { resetFileUrlCache, setBoardHome } from './api';
import { setHostClientFactory } from '../host/factory';
import { setBoardTransport } from './transport';
import { fakeBoard } from './testing';

afterEach(() => {
  setAttachmentBase(null);
  setBoardTransport(undefined);
  setHostClientFactory(undefined);
  resetFileUrlCache();
});

describe('board markdown', () => {
  it('sanitises and renders GFM', () => {
    const html = renderMarkdown('**bold** <script>alert(1)</script>\n- item');
    expect(html).toContain('<strong>bold</strong>');
    expect(html).not.toContain('<script');
    expect(html).toContain('<li>item</li>');
  });

  it('rewrites att: refs to board-relative paths (resolved later through the host blob channel)', () => {
    setAttachmentBase('proj-1');
    expect(attPath('att:T-4/abc12345-shot.png')).toBe('projects/proj-1/attachments/T-4/abc12345-shot.png');
    const html = renderMarkdown('![shot](att:T-4/abc12345-shot.png) and [log](att:T-4/abc12345-run.log)');
    expect(html).toContain('data-att-path="projects/proj-1/attachments/T-4/abc12345-shot.png"');
    expect(html).toContain('class="att-chip att-text"');
    expect(html).not.toContain('att:T-4');
  });

  it('hydrates attachment nodes with object URLs (downloaded by absolute board path)', async () => {
    setAttachmentBase('proj-1');
    setBoardHome('pc', '/home/me/.oino/state/unipi.kanboard');
    setHostClientFactory(() => ({
      hostId: 'pc',
      conn: () => ({ state: 'connected' }),
      retry: async () => {},
      request: async <T,>(op: string, args?: Record<string, unknown>) => {
        expect(op).toBe('blob_get_bytes');
        expect(args?.path).toBe('/home/me/.oino/state/unipi.kanboard/projects/proj-1/attachments/T-4/abc12345-shot.png');
        return { mime: 'image/png', bytes: [1] } as T;
      },
      openEvents: () => ({ close() {} }),
      openSession: () => {
        throw new Error('x');
      },
      openTerm: () => {
        throw new Error('x');
      },
      openLsp: () => {
        throw new Error('x');
      },
    }));
    const created: string[] = [];
    URL.createObjectURL = ((blob: Blob) => {
      created.push(blob.type);
      return 'blob:fake/1';
    }) as typeof URL.createObjectURL;
    const root = document.createElement('div');
    root.innerHTML = renderMarkdown('![shot](att:T-4/abc12345-shot.png)');
    await hydrateAttachments(root, 'pc');
    expect(root.querySelector('img')!.getAttribute('src')).toBe('blob:fake/1');
    expect(created).toEqual(['image/png']);
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
