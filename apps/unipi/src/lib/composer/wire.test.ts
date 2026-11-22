import { describe, expect, it } from 'vitest';
import { attachmentToken, imageToken } from './chips';
import { outgoingMessage, referencedIds } from './wire';

const img = (id: string, extra: Record<string, unknown> = {}) => ({ id, kind: 'image' as const, name: `${id}.png`, status: 'ready' as const, mime: 'image/png', data: `B64${id}`, placed: true, ...extra });
const file = (id: string, extra: Record<string, unknown> = {}) => ({ id, kind: 'file' as const, name: `${id}.pdf`, status: 'ready' as const, mime: 'application/pdf', path: `/host/blobs/${id}`, placed: true, ...extra });
const cmds = new Set(['skill:review']);

describe('outgoingMessage (what the model receives)', () => {
  it('image chips become [image N] at their place, images ordered by the text, not attach order', () => {
    const out = outgoingMessage(`compare ${imageToken('b')} with ${imageToken('a')} please`, [img('a'), img('b')]);
    expect(out.text).toBe('compare [image 1] with [image 2] please');
    expect(out.images.map((i) => i.data)).toEqual(['B64b', 'B64a']);
  });

  it('file chips become [file: name]@<host path> in place', () => {
    const out = outgoingMessage(`read ${attachmentToken('f.pdf', 'f')} then summarize`, [file('f')]);
    expect(out.text).toBe('read [file: f.pdf]@/host/blobs/f then summarize');
  });

  it('a deleted chip leaves its attachment unsent; a never-placed one is still sent after the text', () => {
    const out = outgoingMessage('just text', [img('gone'), file('gone2'), img('old', { placed: false }), file('oldf', { placed: false })]);
    expect(out.text).toBe('just text [image 1]\n\n[file: oldf.pdf]@/host/blobs/oldf');
    expect(out.images.map((i) => i.data)).toEqual(['B64old']);
  });

  it('a $skill chip becomes the /skill: invocation with the attachments resolved', () => {
    const out = outgoingMessage(`$review ${imageToken('a')} looks off`, [img('a')], cmds);
    expect(out).toEqual({ text: '/skill:review [image 1] looks off', images: [{ mimeType: 'image/png', data: 'B64a' }] });
  });

  it('a mode command is sent as the command with its task', () => {
    expect(outgoingMessage('/graph build the site with $review', [], new Set(['graph', 'skill:review'])).text).toBe('/graph build the site with $review');
  });

  it('referencedIds lists the chips in the draft', () => {
    expect([...referencedIds(`${imageToken('a')} x ${attachmentToken('n', 'f')}`)]).toEqual(['a', 'f']);
  });
});
