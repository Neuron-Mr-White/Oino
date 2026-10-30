import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

const openExternal = vi.fn<(url: string) => boolean>();
const copyText = vi.fn<(text: string) => Promise<boolean>>();
vi.mock('../../lib/board/share', () => ({
  openExternal: (url: string) => openExternal(url),
  copyText: (text: string) => copyText(text),
}));

import { MdBlock } from './MdBlock';
import { renderBlocks } from '../../lib/chat/markdown';
import { LinkAnchorMenu, closeLinkMenu } from '../LinkAnchorMenu';

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  closeLinkMenu();
});

describe('MdBlock links (UNI-170)', () => {
  it('tap on an external anchor opens it outside the app', () => {
    openExternal.mockReturnValue(true);
    const html = renderBlocks('[docs](https://example.com/a)', false).join('');
    const r = render(() => <MdBlock html={html} class="chat-block" />);
    const anchor = r.getByText('docs').closest('a')!;
    expect(anchor).toBeTruthy();
    fireEvent.click(anchor);
    expect(openExternal).toHaveBeenCalledWith('https://example.com/a');
  });

  it('long-press on an anchor opens the link menu and stops propagation', () => {
    const html = renderBlocks('[docs](https://example.com/a)', false).join('');
    const parentContextMenu = vi.fn();
    const r = render(() => (
      <div onContextMenu={parentContextMenu}>
        <MdBlock html={html} class="chat-block" />
        <LinkAnchorMenu />
      </div>
    ));
    const anchor = r.getByText('docs').closest('a')!;
    fireEvent.contextMenu(anchor);
    expect(parentContextMenu).not.toHaveBeenCalled();
    expect(screen.getByRole('menu', { name: 'Link actions' })).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: 'Open' })).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: 'Copy link' })).toBeTruthy();
  });

  it('long-press elsewhere does not open the link menu', () => {
    const html = renderBlocks('just text', false).join('');
    const r = render(() => (
      <div>
        <MdBlock html={html} class="chat-block" />
        <LinkAnchorMenu />
      </div>
    ));
    fireEvent.contextMenu(r.container.firstChild as HTMLElement);
    expect(screen.queryByRole('menu', { name: 'Link actions' })).toBeNull();
  });
});
