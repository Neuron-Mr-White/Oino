// @vitest-environment jsdom
/** UNI-276: a command's reply (unipi-command-output) renders in the chat. */
import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { buildRows } from '../../lib/chat/model';
import { UnipiCard } from './Cards';

afterEach(cleanup);

const entry = (data: Record<string, unknown>) => ({ type: 'custom', customType: 'unipi-command-output', id: 'o1', parentId: null, timestamp: '2026-10-12T00:00:00Z', data });

describe('command reply', () => {
  it('is a transcript row', () => {
    const rows = buildRows([entry({ command: "/unipi:goal", text: "No active goal." })] as never, undefined, {}, false);
    expect(rows.some((r) => r.kind === 'card')).toBe(true);
  });
  it('shows the text, and a note for terminal-only screens', () => {
    render(() => (
      <>
        <UnipiCard entry={entry({ command: '/unipi:goal', text: 'No active goal.', level: 'info' }) as never} />
        <UnipiCard entry={entry({ command: '/unipi:settings', screen: true }) as never} />
      </>
    ));
    expect(screen.getByTestId('cmd-reply').textContent).toContain('No active goal.');
    expect(screen.getByTestId('cmd-reply-screen').textContent).toContain('/unipi:settings opened a screen in the terminal.');
  });
});
