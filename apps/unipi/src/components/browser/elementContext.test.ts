import { afterEach, describe, expect, it } from 'vitest';
import { composerDraft, composerSendText, resetComposerDrafts } from '../../lib/composer/drafts';
import { pendingAttachmentsFor, resetAttachments } from '../../lib/composer/attachments';
import { composerKeyOf, elementLabel, elementMarkdown, sendPickToComposer, sendShotToComposer } from './elementContext';
import type { BrowserPick } from '../../lib/browserPane';

const pick: BrowserPick = {
  url: 'http://127.0.0.1:8000/',
  title: 'Probe',
  element: {
    x: 8,
    y: 8,
    width: 120,
    height: 32,
    label: 'button#go.primary',
    detail: {
      url: 'http://127.0.0.1:8000/',
      title: 'Probe page',
      tag: 'button',
      role: 'button',
      name: 'Go now',
      selector: '#go',
      xpath: '/html/body/button',
      text: 'Go now',
      attributes: { id: 'go', class: 'primary', 'aria-label': 'Go now' },
      html: '<button id="go" class="primary">Go now</button>',
    },
  },
  shot: { mime: 'image/png', data: 'iVBORw0KGgo=', width: 120, height: 32 },
};

afterEach(() => {
  resetComposerDrafts();
  resetAttachments();
});

describe('browser → chat context (WP-B8)', () => {
  it('keys the chat composer (draft for a new chat)', () => {
    expect(composerKeyOf('h1', 's1')).toBe('h1/s1');
    expect(composerKeyOf('h1', undefined)).toBe('draft:h1');
  });

  it('labels a pick by tag + accessible name', () => {
    expect(elementLabel(pick)).toBe('<button> Go now');
    expect(elementLabel({ element: { x: 0, y: 0, width: 1, height: 1, label: 'div.card' } })).toBe('<div> div.card');
  });

  it('writes the ZCode field set as Markdown (no duplicate text, no class twice)', () => {
    const md = elementMarkdown(pick);
    expect(md).toContain('**Web element** picked in the browser on “Probe page” (http://127.0.0.1:8000/)');
    expect(md).toContain('- Element: `button#go.primary`, role `button`, name “Go now”');
    expect(md).toContain('- Selector: `#go`');
    expect(md).toContain('- XPath: `/html/body/button`');
    expect(md).not.toContain('- Text:');
    expect(md).toContain('`aria-label="Go now"`');
    expect(md).not.toContain('`class="primary"`');
    expect(md).toContain('```html\n<button id="go" class="primary">Go now</button>\n```');
  });

  it('a pick becomes a chip (flattened into the sent text) plus the crop as an image', () => {
    sendPickToComposer('h1/s1', pick);
    const draft = composerDraft('h1/s1');
    expect(draft.chips).toHaveLength(1);
    expect(draft.chips[0]).toMatchObject({ kind: 'web-element', label: '<button> Go now' });
    expect(composerSendText({ ...draft, text: 'Why is this disabled?' })).toMatch(/\*\*Web element\*\*[\s\S]*Why is this disabled\?$/);
    const images = pendingAttachmentsFor('h1/s1');
    expect(images).toHaveLength(1);
    expect(images[0]).toMatchObject({ kind: 'image', mime: 'image/png', status: 'ready' });
  });

  it('a screenshot is an image attachment named after the page', () => {
    sendShotToComposer('draft:h1', { mime: 'image/jpeg', data: '/9j/4AAQ', width: 10, height: 10 }, 'Probe: page');
    const images = pendingAttachmentsFor('draft:h1');
    expect(images[0]).toMatchObject({ kind: 'image', mime: 'image/jpeg', name: 'Probe  page screenshot.jpg' });
  });
});
