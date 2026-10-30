import { afterEach, describe, expect, it, vi } from 'vitest';

const openExternal = vi.fn<(url: string) => boolean>();
const copyText = vi.fn<(text: string) => Promise<boolean>>();
vi.mock('../board/share', () => ({
  openExternal: (url: string) => openExternal(url),
  copyText: (text: string) => copyText(text),
}));

import { anchorFromEvent, isExternalHref, openLink } from './links';
import { renderBlocks } from '../chat/markdown';

describe('isExternalHref', () => {
  it('accepts http, https and mailto', () => {
    expect(isExternalHref('https://a.b')).toBe(true);
    expect(isExternalHref('http://a.b/x')).toBe(true);
    expect(isExternalHref('mailto:a@b.c')).toBe(true);
  });
  it('rejects other schemes and in-app refs', () => {
    expect(isExternalHref('#section')).toBe(false);
    expect(isExternalHref('javascript:alert(1)')).toBe(false);
    expect(isExternalHref('')).toBe(false);
  });
});

describe('anchorFromEvent', () => {
  it('resolves the anchor from a nested target inside the container', () => {
    const root = document.createElement('div');
    root.innerHTML = '<p><a href="https://a.b"><strong>bold</strong> text</a></p>';
    document.body.appendChild(root);
    const strong = root.querySelector('strong')!;
    const anchor = anchorFromEvent({ target: strong }, root);
    expect(anchor?.getAttribute('href')).toBe('https://a.b');
    root.remove();
  });
  it('returns null for anchors outside the container and non-anchor targets', () => {
    const outer = document.createElement('div');
    const inner = document.createElement('div');
    outer.innerHTML = '<a href="https://a.b">out</a>';
    inner.innerHTML = '<a href="https://c.d">in</a><span>plain</span>';
    outer.appendChild(inner);
    document.body.appendChild(outer);
    const outAnchor = outer.querySelector('a')!;
    expect(anchorFromEvent({ target: outAnchor }, inner)).toBeNull();
    expect(anchorFromEvent({ target: inner.querySelector('span')! }, inner)).toBeNull();
    outer.remove();
  });
});

describe('openLink', () => {
  afterEach(() => vi.clearAllMocks());

  it('opens externally when the platform allows it', async () => {
    openExternal.mockReturnValue(true);
    await expect(openLink('https://a.b')).resolves.toBe('opened');
    expect(copyText).not.toHaveBeenCalled();
  });
  it('falls back to copying', async () => {
    openExternal.mockReturnValue(false);
    copyText.mockResolvedValue(true);
    await expect(openLink('https://a.b')).resolves.toBe('copied');
  });
  it('fails when neither works', async () => {
    openExternal.mockReturnValue(false);
    copyText.mockResolvedValue(false);
    await expect(openLink('https://a.b')).resolves.toBe('failed');
  });
});

describe('markdown autolinks', () => {
  it('bare URLs render as anchors (so taps are handled)', () => {
    const html = renderBlocks('see https://example.com/x now', false).join('');
    expect(html).toContain('<a href="https://example.com/x"');
  });
});
