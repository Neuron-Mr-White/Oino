/** OIN-97 in kanboard / report markdown: the same inline-code right-click Copy as the chat. */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

const copyText = vi.fn<(text: string) => Promise<boolean>>();
vi.mock('../../lib/board/share', () => ({
  openExternal: () => true,
  copyText: (text: string) => copyText(text),
  saveFile: async () => 'unsupported' as const,
}));

import { Markdown } from './Markdown';
import { LinkAnchorMenu, closeLinkMenu } from '../LinkAnchorMenu';
import { setDesktopOverride } from '../../lib/layout';

afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  vi.clearAllMocks();
  closeLinkMenu();
});

describe('board Markdown inline code (OIN-97)', () => {
  it('right-click on inline code offers Copy of the inner text, and no ancestor menu opens', () => {
    setDesktopOverride(true);
    copyText.mockResolvedValue(true);
    const ancestor = vi.fn();
    const r = render(() => (
      <div ref={(el) => el.addEventListener('contextmenu', ancestor)}>
        <Markdown text={'open `crates/x/src/lib.rs` now\n\n```sh\nnpm test\n```'} />
        <LinkAnchorMenu />
      </div>
    ));
    fireEvent.contextMenu(r.container.querySelector('p code')!);
    expect(ancestor).not.toHaveBeenCalled();
    expect(screen.getByRole('menu', { name: 'Code actions' })).toBeTruthy();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy' }));
    expect(copyText).toHaveBeenCalledWith('crates/x/src/lib.rs');
    // a fenced block gets none
    closeLinkMenu();
    fireEvent.contextMenu(r.container.querySelector('pre code')!);
    expect(screen.queryByRole('menu', { name: 'Code actions' })).toBeNull();
  });
});
