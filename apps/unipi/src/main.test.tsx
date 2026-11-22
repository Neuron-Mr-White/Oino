import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, it, expect } from 'vitest';
import { Root } from './main';

afterEach(cleanup);
it('Chat tab is a real link to /chat (UNI-113)', () => {
  render(() => <Root />);
  const chat = screen.getByRole('link', { name: /Chat/ });
  expect(chat).not.toHaveAttribute('aria-disabled');
  expect(chat.getAttribute('href')).toMatch(/#\/chat$/);
});

// UNI-155's bottom-nav Files tab is gone: Files is a side-pane tab only
// (Z-101, WP-A20); the phone reaches it through the chat header.
it('Files is not a bottom-nav tab (Z-101)', () => {
  render(() => <Root />);
  expect(screen.queryByRole('link', { name: /Files/ })).toBeNull();
});

it('web mode (UNI-117): Chat shows the standard "needs the app" notice', async () => {
  const { setPlatformOverride } = await import('./lib/platform');
  setPlatformOverride('web');
  location.hash = '#/chat';
  try {
    render(() => <Root />);
    const notice = await screen.findByTestId('unavailable-notice');
    expect(notice).toHaveTextContent('Not available in the web version');
  } finally {
    setPlatformOverride(undefined);
    location.hash = '';
  }
});
