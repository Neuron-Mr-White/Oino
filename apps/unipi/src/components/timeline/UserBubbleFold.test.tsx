// @vitest-environment jsdom
/**
 * User bubble fold + Edit (WP-A7, Z-29/Z-28): a 120 px body with a centred
 * expand/collapse button, the chips outside the folded body, the fold reset
 * when the row is reused for another message, and the Edit pencil.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { RefChat } from '../../lib/chat/pathStat';
import type { ImageRef } from '../../lib/chat/model';
import { UserBubble } from './UserBubble';

let bodyHeight = 400;
beforeEach(() => {
  bodyHeight = 400;
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', {
    configurable: true,
    get() {
      return this.classList?.contains('tl-bubble-body') ? bodyHeight : 0;
    },
  });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const commands = () => new Set<string>();
const mkChat = () =>
  ({
    hostId: 'h1',
    session: '1',
    cwd: () => '/proj',
    statPaths: vi.fn(async (paths: string[]) => paths.map((path) => ({ path, resolved: path, kind: 'missing' }))),
  }) as unknown as RefChat;

const image = (): ImageRef => ({ mime: 'image/png', data: 'ZGF0YQ==' });

describe('user bubble fold (Z-29)', () => {
  it('caps at 120 px with a fade and expands/collapses from the centred button', async () => {
    const r = render(() => <UserBubble chat={mkChat()} text={'long message\n'.repeat(30)} commands={commands} />);
    const bubble = await waitFor(() => r.container.querySelector('.tl-bubble') as HTMLElement);
    const body = bubble.querySelector('.tl-bubble-body') as HTMLElement;
    await waitFor(() => expect(r.getByTestId('tl-bubble-fold')).toBeTruthy());
    expect(bubble.hasAttribute('data-folded')).toBe(true);
    expect(body.style.maxHeight).toBe('120px');
    expect(r.getByTestId('tl-bubble-fold').getAttribute('aria-expanded')).toBe('false');

    fireEvent.click(r.getByTestId('tl-bubble-fold'));
    await waitFor(() => expect(r.getByTestId('tl-bubble-fold').getAttribute('aria-expanded')).toBe('true'));
    expect(bubble.hasAttribute('data-folded')).toBe(false);
    expect(body.style.maxHeight).toBe('400px');

    fireEvent.click(r.getByTestId('tl-bubble-fold'));
    await waitFor(() => expect(r.getByTestId('tl-bubble-fold').getAttribute('aria-expanded')).toBe('false'));
    expect(body.style.maxHeight).toBe('120px');
  });

  it('shows no fold button when the body fits', async () => {
    bodyHeight = 80;
    const r = render(() => <UserBubble chat={mkChat()} text="short" commands={commands} />);
    await waitFor(() => expect(r.container.querySelector('.tl-bubble-body')).toBeTruthy());
    expect(r.queryByTestId('tl-bubble-fold')).toBeNull();
  });

  it('resets to collapsed when the virtualizer reuses the row for another message', async () => {
    const [text, setText] = createSignal(`first message\n${'more text\n'.repeat(20)}`);
    const r = render(() => <UserBubble chat={mkChat()} text={text()} commands={commands} />);
    await waitFor(() => expect(r.getByTestId('tl-bubble-fold')).toBeTruthy());
    fireEvent.click(r.getByTestId('tl-bubble-fold'));
    await waitFor(() => expect(r.getByTestId('tl-bubble-fold').getAttribute('aria-expanded')).toBe('true'));
    setText(`second message\n${'other text\n'.repeat(20)}`);
    await waitFor(() => expect(r.getByTestId('tl-bubble-fold').getAttribute('aria-expanded')).toBe('false'));
  });

  it('keeps image chips outside the folded body', async () => {
    const r = render(() => <UserBubble chat={mkChat()} text={'long message\n'.repeat(30)} images={[image()]} commands={commands} />);
    const bubble = await waitFor(() => r.container.querySelector('.tl-bubble') as HTMLElement);
    await waitFor(() => expect(r.getByTestId('tl-bubble-fold')).toBeTruthy());
    expect(bubble.querySelector('.chat-image-chip')).toBeNull();
    expect(r.container.querySelector('.chat-image-chip')).toBeTruthy();
  });

  it('shows the Edit pencil and hands back its anchor', async () => {
    const onEdit = vi.fn();
    const r = render(() => <UserBubble chat={mkChat()} text="edit me" commands={commands} onEdit={onEdit} />);
    const edit = await waitFor(() => r.getByTestId('tl-user-edit'));
    fireEvent.click(edit);
    expect(onEdit).toHaveBeenCalledTimes(1);
    expect(onEdit.mock.calls[0]![0]).toMatchObject({ width: expect.any(Number), top: expect.any(Number) });
  });
});
