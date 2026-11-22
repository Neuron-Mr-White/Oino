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
import { setDesktopOverride } from '../../lib/layout';
import { actionNotice, attachMessageMenu, clearActionNotice, closeMessageMenu, currentMessageMenu } from '../../lib/timeline/actions';

afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  vi.clearAllMocks();
  closeLinkMenu();
  closeMessageMenu();
  clearActionNotice();
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

describe('MdBlock inline code context menu (OIN-97)', () => {
  const html = renderBlocks('run `npm  run  dev` now\n\n```sh\nnpm test\n```', false).join('');

  it('right-click on inline code offers Copy of its exact inner text', async () => {
    setDesktopOverride(true);
    copyText.mockResolvedValue(true);
    const parentContextMenu = vi.fn();
    const r = render(() => (
      <div onContextMenu={parentContextMenu}>
        <MdBlock html={html} class="chat-block" />
        <LinkAnchorMenu />
      </div>
    ));
    const code = r.container.querySelector('p code')!;
    expect(code.textContent).toBe('npm  run  dev');
    fireEvent.contextMenu(code);
    expect(parentContextMenu).not.toHaveBeenCalled();
    expect(screen.getByRole('menu', { name: 'Code actions' })).toBeTruthy();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy' }));
    expect(copyText).toHaveBeenCalledWith('npm  run  dev');
  });

  it('a fenced code block and plain text get no code menu', () => {
    setDesktopOverride(true);
    const r = render(() => (
      <div>
        <MdBlock html={html} class="chat-block" />
        <LinkAnchorMenu />
      </div>
    ));
    fireEvent.contextMenu(r.container.querySelector('pre code')!);
    fireEvent.contextMenu(r.container.querySelector('p')!);
    expect(screen.queryByRole('menu', { name: 'Code actions' })).toBeNull();
  });

  it('a phone long-press keeps native selection: no code menu', () => {
    setDesktopOverride(false);
    const r = render(() => (
      <div>
        <MdBlock html={html} class="chat-block" />
        <LinkAnchorMenu />
      </div>
    ));
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    r.container.querySelector('p code')!.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
    expect(screen.queryByRole('menu', { name: 'Code actions' })).toBeNull();
  });

  it('a NATIVE ancestor listener (the message menu) never sees it: ONE menu, and the copied toast shows', async () => {
    // Solid delegates `onContextMenu` to the document, so a handler there is too late to stop an element listener
    // above it — the real chat opened the message menu (Quote / Fork / Retry …) on top of the code menu.
    setDesktopOverride(true);
    copyText.mockResolvedValue(true);
    const ancestor = vi.fn();
    const r = render(() => (
      <div ref={(el) => { attachMessageMenu(el, () => ({ text: 'x', role: 'assistant', turnId: 't' })); el.addEventListener('contextmenu', ancestor); }}>
        <MdBlock html={html} class="chat-block" />
        <LinkAnchorMenu />
      </div>
    ));
    fireEvent.contextMenu(r.container.querySelector('p code')!);
    expect(ancestor).not.toHaveBeenCalled();
    expect(currentMessageMenu()).toBeUndefined();
    expect(screen.getAllByRole('menu')).toHaveLength(1);
    expect(screen.getByRole('menu', { name: 'Code actions' })).toBeTruthy();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy' }));
    expect(copyText).toHaveBeenCalledWith('npm  run  dev');
    await vi.waitFor(() => expect(actionNotice()).toBe('Copied'));
  });

  it('plain text still opens the message menu, and a text selection keeps the native menu', () => {
    setDesktopOverride(true);
    const r = render(() => (
      <div ref={(el) => attachMessageMenu(el, () => ({ text: 'x', role: 'assistant', turnId: 't' }))}>
        <MdBlock html={html} class="chat-block" />
        <LinkAnchorMenu />
      </div>
    ));
    fireEvent.contextMenu(r.container.querySelector('p')!);
    expect(currentMessageMenu()).toBeDefined();
    closeMessageMenu();
    // a live selection: neither of our menus (the browser's own menu offers Copy of the selection)
    const sel = window.getSelection()!;
    const range = document.createRange();
    range.selectNodeContents(r.container.querySelector('p code')!);
    sel.removeAllRanges();
    sel.addRange(range);
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    r.container.querySelector('p code')!.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
    expect(screen.queryByRole('menu', { name: 'Code actions' })).toBeNull();
    sel.removeAllRanges();
  });
});
