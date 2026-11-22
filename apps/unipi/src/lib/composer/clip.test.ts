import { describe, expect, it } from 'vitest';
import { attachmentToken, imageToken } from './chips';
import { CLIP_MIME, clipIds, decodeClip, encodeClip, fileOfDataUrl, pastedText, plainOf, type ComposerClip } from './clip';

const clip: ComposerClip = {
  v: 1,
  text: `see ${imageToken('i1')} and ${attachmentToken('a.pdf', 'f1')} ✓`,
  attachments: [
    { id: 'i1', kind: 'image', name: 'shot.png', mime: 'image/png', data: 'data:image/png;base64,QUJD' },
    { id: 'f1', kind: 'file', name: 'a.pdf', mime: 'application/pdf', path: '/blobs/a.pdf' },
  ],
};
const transfer = (data: Record<string, string>) => ({ getData: (t: string) => data[t] ?? '' });

describe('rich composer clipboard', () => {
  it('names the chips and reads them for other apps', () => {
    expect(clipIds(clip.text)).toEqual(['i1', 'f1']);
    expect(plainOf(clip.text, clip.attachments)).toBe('see [image] and @/blobs/a.pdf ✓');
  });
  it('round-trips through the custom type and through the HTML fallback (unicode included)', () => {
    const out = encodeClip(clip);
    expect(decodeClip(transfer({ [CLIP_MIME]: out.json }))).toEqual(clip);
    expect(decodeClip(transfer({ 'text/html': `<meta charset="utf-8">${out.html}` }))).toEqual(clip);
    expect(decodeClip(transfer({ 'text/plain': 'just text' }))).toBeUndefined();
    expect(decodeClip(transfer({ [CLIP_MIME]: '{"v":2}' }))).toBeUndefined();
  });
  it('keeps chips the box still holds, remaps re-attached ones, and degrades the rest', () => {
    expect(pastedText(clip, () => true, new Map())).toBe(clip.text);
    expect(pastedText(clip, () => false, new Map([['i1', 'i9']]))).toBe(`see ${imageToken('i9')} and @/blobs/a.pdf ✓`);
    expect(pastedText({ ...clip, attachments: [] }, () => false, new Map())).toBe('see [image] and [file: a.pdf] ✓');
  });
  it('turns a data URL back into a file', async () => {
    const f = fileOfDataUrl('data:image/png;base64,QUJD', 'x.png', 'image/png')!;
    expect(f.type).toBe('image/png');
    expect(f.name).toBe('x.png');
    expect(await f.text()).toBe('ABC');
  });
});
