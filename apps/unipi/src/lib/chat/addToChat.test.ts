// @vitest-environment jsdom
/** "Add to chat" (owner spec 2026-11-03): images come back as attachments (inline chips), files
 *  as a re-reference to the blob already on the PC — never a re-upload. */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { addToChat, fileItemToken } from './addToChat';
import { composerDraft, resetComposerDrafts } from '../composer/drafts';
import { drainAttachments, pendingAttachmentsFor } from '../composer/attachments';
import type { RefChat } from './pathStat';

vi.mock('../imageShrink', () => ({
  MAX_SIDE: 1568,
  QUALITY: 0.8,
  INLINE_MAX_BYTES: 700 * 1024,
  shrinkImage: async (file: Blob) => ({ blob: file, mime: 'image/png', width: 1, height: 1 }),
  blobToBase64: async () => 'QUJD',
}));

const chat = { hostId: 'pc', session: 's1', statPaths: async () => [] } as unknown as RefChat;
afterEach(() => resetComposerDrafts());

describe('addToChat', () => {
  it('a sent file goes back as [file: name]@<host path> (no upload)', async () => {
    await addToChat(chat, { kind: 'file', path: '/host/blobs/ab', name: 'report.pdf' });
    expect(composerDraft('pc/s1').text).toBe('[file: report.pdf]@/host/blobs/ab ');
    expect(pendingAttachmentsFor('pc/s1')).toHaveLength(0);
    expect(fileItemToken({ path: '/p', name: 'a]b' })).toBe('[file: a_b]@/p');
  });

  it('a non-image tool attachment is a file reference too', async () => {
    await addToChat(chat, { kind: 'attachment', path: '/w/out.csv', mime: 'text/csv', name: 'out.csv' });
    expect(composerDraft('pc/s1').text).toBe('[file: out.csv]@/w/out.csv ');
  });

  it('an image in the chat becomes a pending image (its inline chip is placed by the composer)', async () => {
    await addToChat(chat, { kind: 'image', image: { mime: 'image/png', data: 'QUJD' }, name: 'image 1.png' });
    await drainAttachments('pc/s1');
    const items = pendingAttachmentsFor('pc/s1');
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ kind: 'image', status: 'ready', data: 'QUJD', name: 'image 1.png' });
  });

  it('refuses without a session', async () => {
    await expect(addToChat({ ...chat, session: undefined } as RefChat, { kind: 'file', path: '/p', name: 'p' })).rejects.toThrow();
  });
});
