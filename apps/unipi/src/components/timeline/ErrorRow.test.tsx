// @vitest-environment jsdom
/**
 * ErrorRow (WP-A9, Z-48): the red row carries the error's first line, the
 * "Details" disclosure shows the **raw** provider error, and Copy puts it on
 * the clipboard (glyph → "Copied" for 1.2 s, Z-93).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ErrorRow, TimelineError, errorHeadline, errorHint } from './ErrorRow';
import type { TimelineCtx } from './context';
import type { Turn } from '../../lib/session/turns';

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

describe('TimelineError', () => {
  const errTurn = (id: string, keys: string[]): Turn =>
    ({ turnId: id, segments: [{ work: keys.map((key) => ({ kind: 'error', key, text: 'rate limited' })) }] }) as unknown as Turn;
  const ctxOf = (turns: Turn[]) => ({ sessionId: () => 's1', session: { turns: () => turns } }) as unknown as TimelineCtx;

  it('the newest error is the full card, with the folded count', () => {
    render(() => <TimelineError text="rate limited" ctx={ctxOf([errTurn('t1', ['a', 'b'])])} itemKey="b" count={4} />);
    expect(screen.getByTestId('tl-error')).toHaveTextContent('rate limited ×4');
    expect(screen.queryByTestId('tl-error-line')).toBeNull();
  });

  it('an older error is one line that expands in place to the card and collapses again', () => {
    render(() => <TimelineError text="rate limited" ctx={ctxOf([errTurn('t1', ['a', 'b'])])} itemKey="a" count={3} />);
    const line = screen.getByTestId('tl-error-line');
    expect(line).toHaveTextContent('rate limited×3');
    expect(screen.queryByTestId('tl-error')).toBeNull();
    fireEvent.click(line);
    expect(screen.queryByTestId('tl-error-line')).toBeNull();
    expect(screen.getByTestId('tl-error')).toHaveTextContent('rate limited ×3');
    fireEvent.click(screen.getByText('rate limited'));
    expect(screen.getByTestId('tl-error-line')).toBeInTheDocument();
    expect(screen.queryByTestId('tl-error')).toBeNull();
  });
});
