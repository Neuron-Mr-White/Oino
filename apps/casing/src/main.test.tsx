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

it('Files tab is a real link to /files (UNI-155)', () => {
  render(() => <Root />);
  const files = screen.getByRole('link', { name: /Files/ });
  expect(files).not.toHaveAttribute('aria-disabled');
  expect(files.getAttribute('href')).toMatch(/#\/files$/);
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
