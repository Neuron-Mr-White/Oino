// @vitest-environment jsdom
/**
 * UNI-226: chat menus mounted the way ChatView mounts them —
 * `<Show when={signal()}>{(m) => <Menu … />}</Show>` — must still run the
 * picked item after `onDismiss()` unmounts them. Reading a prop after the
 * dismiss used to throw "Stale read from <Show>", so every item of the full
 * message menu (⋯ / right-click) and the header ⋯ Running / Context /
 * Session info rows did nothing, while the icon action bar worked.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createSignal, Show } from 'solid-js';
import { ChatHeaderMenu, MessageMenu } from './SessionMenus';

vi.mock('../../lib/board/share', () => ({ copyText: vi.fn(async () => true), openExternal: () => false }));
import { copyText } from '../../lib/board/share';

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

const rect = () => new DOMRect(10, 10, 20, 20);

function renderMessageMenu(extra: { onStartFrom?: () => void; onForkFrom?: () => void; onRetry?: () => void } = {}) {
  const onQuote = vi.fn();
  const [menu, setMenu] = createSignal<{ text: string } | null>({ text: 'hello there' });
  render(() => (
    <Show when={menu()}>
      {(m) => (
        <MessageMenu
          anchor={rect()}
          text={m().text}
          entryId="e1"
          onDismiss={() => setMenu(null)}
          onHaptic={() => {}}
          onQuote={onQuote}
          {...extra}
        />
      )}
    </Show>
  ));
  return { onQuote, menu };
}

describe('MessageMenu inside <Show> (UNI-226)', () => {
  it('Quote closes the menu and still quotes the text', () => {
    const { onQuote, menu } = renderMessageMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: /Quote/ }));
    expect(menu()).toBeNull();
    expect(onQuote).toHaveBeenCalledWith('hello there');
  });

  it('Copy copies the text after closing', () => {
    renderMessageMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: /Copy/ }));
    expect(copyText).toHaveBeenCalledWith('hello there');
  });

  it('Start from here / Fork from here / Retry run their callbacks', () => {
    const onStartFrom = vi.fn();
    const onForkFrom = vi.fn();
    const onRetry = vi.fn();
    for (const [name, fn] of [[/Start from here/, onStartFrom], [/Fork from here/, onForkFrom], [/Retry/, onRetry]] as const) {
      renderMessageMenu({ onStartFrom, onForkFrom, onRetry });
      fireEvent.click(screen.getByRole('menuitem', { name }));
      expect(fn).toHaveBeenCalledTimes(1);
      cleanup();
    }
  });
});

describe('ChatHeaderMenu inside <Show> (UNI-226)', () => {
  it.each([
    ['Running', 'running'],
    ['Context', 'context'],
    ['Session info', 'info'],
  ])('%s opens its Session sheet section after closing', (label, section) => {
    const onOpenSection = vi.fn();
    const [anchor, setAnchor] = createSignal<DOMRect | null>(rect());
    render(() => (
      <Show when={anchor()}>
        {(a) => (
          <ChatHeaderMenu
            anchor={a()}
            onDismiss={() => setAnchor(null)}
            onNew={() => {}}
           
            onTree={() => {}}
            onRename={() => {}}
            canRename
            onOpenSection={onOpenSection}
          />
        )}
      </Show>
    ));
    fireEvent.click(screen.getByRole('menuitem', { name: new RegExp(`^${label}`) }));
    expect(anchor()).toBeNull();
    expect(onOpenSection).toHaveBeenCalledWith(section);
  });
});
