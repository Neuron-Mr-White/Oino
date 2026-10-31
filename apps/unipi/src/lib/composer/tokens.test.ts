// @vitest-environment jsdom
import { createRoot, createSignal } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readyImages, resetAttachments } from './attachments';
import { composerDraft, resetComposerDrafts } from './drafts';
import {
  connectEditorTokens,
  insertEditorToken,
  parseEditorToken,
  resetEditorTokens,
  subscribeEditorTokens,
  type EditorToken,
} from './tokens';

afterEach(() => {
  resetEditorTokens();
  resetComposerDrafts();
  resetAttachments();
});

describe('editor token insertion API (P0-08)', () => {
  it('parses both nested {token} and flat token payloads', () => {
    expect(
      parseEditorToken({
        method: 'insertEditorToken',
        token: { tokenId: 't1', kind: 'file', label: 'effects.ts', path: 'src/ui/slots/effects.ts' },
      }),
    ).toEqual({
      tokenId: 't1',
      kind: 'file',
      label: 'effects.ts',
      path: 'src/ui/slots/effects.ts',
    });
    expect(
      parseEditorToken({
        method: 'insert_editor_token',
        kind: 'image',
        label: 'shot.png',
        data: { mimeType: 'image/png', data: 'YWJj' },
      }),
    ).toEqual({
      kind: 'image',
      label: 'shot.png',
      data: { mimeType: 'image/png', data: 'YWJj' },
    });
  });

  it('writes text, path, chip and image tokens to the session draft store when unmounted', () => {
    const key = 'local/s1';
    insertEditorToken(key, { kind: 'text', label: 'Explain' });
    insertEditorToken(key, { kind: 'file', label: 'my file.ts', path: 'src/my file.ts' });
    insertEditorToken(key, { tokenId: 'c9', kind: 'element', label: '<button.send>', markdown: '```html\n<button class="send"/>\n```' });
    insertEditorToken(key, { kind: 'image', label: 'shot.png', data: { mimeType: 'image/png', data: 'YWJj' } });

    const draft = composerDraft(key);
    expect(draft.text).toBe('Explain @"src/my file.ts" ');
    expect(draft.chips).toEqual([{ id: 'c9', kind: 'element', label: '<button.send>', markdown: '```html\n<button class="send"/>\n```' }]);
    expect(readyImages(key)).toEqual([{ mimeType: 'image/png', data: 'YWJj' }]);
  });

  it('delivers tokens to live subscribers without double-appending to the draft', () => {
    const key = 'local/s2';
    const seen: EditorToken[] = [];
    const off = subscribeEditorTokens(key, (tok) => seen.push(tok));
    insertEditorToken(key, { kind: 'file', label: 'a.ts', path: 'src/a.ts' });
    expect(seen).toEqual([{ kind: 'file', label: 'a.ts', path: 'src/a.ts' }]);
    expect(composerDraft(key).text).toBe('');
    off();
  });

  it('connectEditorTokens wires a reactive session key to caret insertion', () => {
    const inserted = vi.fn();
    let setSessionKey!: (next: string) => void;
    const dispose = createRoot((d) => {
      const [key, setKey] = createSignal('local/a');
      setSessionKey = setKey;
      connectEditorTokens(key, inserted);
      return d;
    });
    insertEditorToken('local/a', { kind: 'file', label: 'x.ts', path: 'src/x.ts' });
    expect(inserted).toHaveBeenCalledWith('@src/x.ts ');
    setSessionKey('local/b');
    insertEditorToken('local/b', { kind: 'text', label: 'hello' });
    expect(inserted).toHaveBeenLastCalledWith('hello');
    dispose();
  });
});
