// @vitest-environment jsdom
/**
 * ErrorRow (WP-A9, Z-48): the red row carries the error's first line, the
 * "Details" disclosure shows the **raw** provider error, and Copy puts it on
 * the clipboard (glyph → "Copied" for 1.2 s, Z-93).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ErrorRow, errorHeadline, errorHint } from './ErrorRow';

const copy = vi.fn<(text: string) => Promise<boolean>>(async () => true);
vi.mock('../../lib/board/share', () => ({ copyText: (text: string) => copy(text) }));

const RAW = ['overloaded_error: the model is overloaded (529)', '', 'request-id: req_011Cb3f2a9e04b1c', 'retry-after: 12s'].join('\n');

afterEach(() => {
  cleanup();
  copy.mockClear();
});

describe('ErrorRow', () => {
  it('takes its line from the first non-empty line of the raw error', () => {
    expect(errorHeadline(RAW)).toBe('overloaded_error: the model is overloaded (529)');
    expect(errorHeadline('\n\n  failed  \nmore')).toBe('failed');
    expect(errorHeadline('')).toBe('The turn failed.');
  });

  it('D-24: Details names the model and a next step for an auth error', () => {
    expect(errorHint('401: {"message":"User not found.","code":401}')).toContain('Settings › Providers');
    expect(errorHint('some odd failure')).toBeUndefined();
    render(() => <ErrorRow text={'401: {"message":"User not found.","code":401}'} model="openrouter · kimi-k2.6" />);
    fireEvent.click(screen.getByTestId('tl-error-details'));
    expect(screen.getByTestId('tl-error-facts').textContent).toContain('openrouter · kimi-k2.6');
  });

  it('reveals the raw error behind Details and copies it', async () => {
    render(() => <ErrorRow text={RAW} />);
    expect(screen.getByTestId('tl-error')).toHaveTextContent('the model is overloaded');
    expect(screen.queryByTestId('tl-error-raw')).toBeNull();
    expect(screen.getByTestId('tl-error-details')).toHaveAttribute('aria-expanded', 'false');

    fireEvent.click(screen.getByTestId('tl-error-details'));
    expect(screen.getByTestId('tl-error-details')).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByTestId('tl-error-raw').textContent).toBe(RAW);

    fireEvent.click(screen.getByLabelText('Copy the error'));
    await waitFor(() => expect(copy).toHaveBeenCalledWith(RAW));
    expect(await screen.findByText('Copied')).toBeInTheDocument();
  });
});
