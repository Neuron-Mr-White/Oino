// @vitest-environment jsdom
/** Right-click on a chat image or sent file opens "Add to chat" (owner spec 2026-11-03). */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { FileMentionChipRow, ImageChipRow } from './ImageChip';
import { ChatItemMenu } from './ChatItemMenu';
import { chatItemMenu, closeChatItemMenu } from '../../lib/chat/addToChat';
import { composerDraft, resetComposerDrafts } from '../../lib/composer/drafts';
import type { RefChat } from '../../lib/chat/pathStat';

const chat = { hostId: 'pc', session: 's1', statPaths: async () => [] } as unknown as RefChat;
afterEach(() => {
  cleanup();
  closeChatItemMenu();
  resetComposerDrafts();
});

describe('Add to chat menu', () => {
  it('right-click on a sent image opens it for that image', () => {
    const view = render(() => <ImageChipRow chat={chat} images={[{ mime: 'image/png', data: 'QUJD' }]} />);
    fireEvent.contextMenu(view.container.querySelector('.chat-image-chip')!);
    expect(chatItemMenu()?.item).toMatchObject({ kind: 'image', name: 'image 1.png' });
  });

  it('"Add to chat" on a sent file puts its [file: …] chip back in the draft', async () => {
    const view = render(() => (
      <>
        <FileMentionChipRow chat={chat} mentions={[{ name: 'a.pdf', path: '/host/blobs/a', start: 0, end: 1 }]} />
        <ChatItemMenu />
      </>
    ));
    fireEvent.contextMenu(view.container.querySelector('.chat-attachment-chip')!);
    const add = await waitFor(() => document.querySelector('[data-testid="chat-item-add"]') as HTMLElement);
    fireEvent.click(add);
    await waitFor(() => expect(composerDraft('pc/s1').text).toBe('[file: a.pdf]@/host/blobs/a '));
  });
});
