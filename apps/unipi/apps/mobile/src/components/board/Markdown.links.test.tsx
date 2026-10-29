import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';

const openExternal = vi.fn<(url: string) => boolean>();
vi.mock('../../lib/board/share', () => ({
  openExternal: (url: string) => openExternal(url),
  copyText: async () => true,
  saveFile: async () => 'unsupported' as const,
}));

import { Markdown } from './Markdown';

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('board Markdown links (UNI-170)', () => {
  it('tap on an external anchor opens it outside the app', () => {
    openExternal.mockReturnValue(true);
    const r = render(() => <Markdown text="see [the docs](https://example.com/b) please" />);
    fireEvent.click(r.getByText('the docs').closest('a')!);
    expect(openExternal).toHaveBeenCalledWith('https://example.com/b');
  });
});
