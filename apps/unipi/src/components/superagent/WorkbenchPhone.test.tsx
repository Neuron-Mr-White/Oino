// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagentReview } from '../../dev/mock/scenarios/superagent-review';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { boardUi } from '../../lib/board/ui';
import { resetPendingMoves } from '../../lib/board/pendingMoves';
import { endQueue, taskOrigin } from '../../lib/board/taskOrigin';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));

const spec = () => { const host = superagentReview.hosts![0]!; return { ...superagentReview, hosts: [{ ...host, board: structuredClone(host.board) }] }; };
/** jsdom has no stylesheet pipeline: load the workbench's own phone rules (the ones jsdom's selector engine can match). */
function loadPhoneRules() {
  const css = readFileSync('src/components/superagent/superagent.css', 'utf8');
  const rules = css.split('\n').filter((line) => /^(html\[data-shell=phone\] )?\.sa-review-(body|list-pane|detail)/.test(line) && !line.includes(':has'));
  const style = document.createElement('style');
  style.textContent = rules.join('\n');
  document.head.append(style);
  document.documentElement.dataset.shell = 'phone';
  return style;
}
afterEach(() => { cleanup(); resetPendingMoves(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); boardUi().closeTask(); endQueue(); document.querySelectorAll('style').forEach((s) => s.remove()); location.hash = ''; });

describe('phone: the review workbench always shows something', () => {
  it('shows "Queue done" (not a blank sheet) once the last ticket is reviewed', async () => {
    setHostClientFactory(createMockHost(spec()).client);
    setPlatformOverride('tauri-mobile'); setDesktopOverride(false);
    localStorage.setItem('unipi.board.last', JSON.stringify({ hostId: 'pc-coffee', projects: { 'pc-coffee': 'example-app' } }));
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    const style = loadPhoneRules();
    await screen.findByTestId('superagent-page');
    fireEvent.click(await screen.findByLabelText('Open review (7)'));
    fireEvent.click(await screen.findByLabelText('Open HP-3'));
    await screen.findByTestId('task-detail');
    for (let i = 0; i < 7; i += 1) {
      fireEvent.click(await screen.findByText('Mark done'));
      if (i < 6) await waitFor(() => expect(taskOrigin()?.items.length).toBe(6 - i));
    }
    const done = await screen.findByText('Queue done');
    expect(style.isConnected).toBe(true);
    const shown = (el: Element) => getComputedStyle(el).display !== 'none';
    expect(shown(done.closest('.sa-review-detail')!)).toBe(true);
    // The (empty) list does not push the message off the sheet's edge.
    expect(shown(document.querySelector('.sa-review-list-pane')!)).toBe(false);
  });
});
