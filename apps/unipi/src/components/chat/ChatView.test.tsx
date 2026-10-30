// @vitest-environment jsdom
/**
 * ChatView pieces: UNI-206 (the tail `RunningRow` must not repeat the
 * "Running · <cmd>" line a live ProcessGroup/prose row already shows at
 * the end of the transcript — `compact` drops the title/detail and keeps
 * only the dot + ticking elapsed timer) and UNI-207 (`isMsgTapIgnored`:
 * which taps inside a message row must NOT toggle the selection action bar).
 */
import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { isMsgTapIgnored, restoreIntoDraft, RunningRow } from './ChatView';

afterEach(cleanup);

describe('RunningRow (UNI-206)', () => {
  it('shows the title and detail by default', () => {
    render(() => <RunningRow since={Date.now() - 1000} label={{ title: 'Running', detail: 'npm test' }} />);
    expect(screen.getByText('Running')).toBeTruthy();
    expect(screen.getByText('npm test')).toBeTruthy();
  });

  it('compact: drops the title/detail duplicate, keeps the dot + elapsed timer', () => {
    render(() => <RunningRow since={Date.now() - 1000} label={{ title: 'Running', detail: 'npm test' }} compact />);
    expect(screen.queryByText('npm test')).toBeNull();
    // A ticking timer still shows ("Working" placeholder + elapsed time), not the detail text.
    expect(screen.getByText('Working')).toBeTruthy();
  });
});

describe('isMsgTapIgnored (UNI-207)', () => {
  afterEach(() => (document.body.innerHTML = ''));

  it('ignores taps on links, code, buttons, image chips and task refs', () => {
    document.body.innerHTML = `
      <div class="chat-row-touch">
        <a href="https://x"><span id="a">link</span></a>
        <code><span id="b">code</span></code>
        <button type="button"><span id="c">btn</span></button>
        <button type="button" class="chat-image-chip"><span id="d">img</span></button>
        <span class="task-ref"><span id="e">ref</span></span>
        <span id="f">plain text</span>
      </div>`;
    for (const id of ['a', 'b', 'c', 'd', 'e']) {
      expect(isMsgTapIgnored(document.getElementById(id)!)).toBe(true);
    }
    expect(isMsgTapIgnored(document.getElementById('f')!)).toBe(false);
  });
});

describe('restoreIntoDraft (UNI-290 "Start from here")', () => {
  it('an empty (or blank) draft is replaced right away', () => {
    for (const draft of ['', '  \n ']) {
      const apply = vi.fn();
      const ask = vi.fn();
      restoreIntoDraft(draft, apply, ask);
      expect(apply).toHaveBeenCalledTimes(1);
      expect(ask).not.toHaveBeenCalled();
    }
  });

  it('a draft with text asks "Replace your draft?" first; confirming applies', async () => {
    const apply = vi.fn();
    const ask = vi.fn();
    restoreIntoDraft('half-typed idea', apply, ask);
    expect(apply).not.toHaveBeenCalled();
    expect(ask).toHaveBeenCalledTimes(1);
    const req = ask.mock.calls[0]![0];
    expect(req.title).toBe('Replace your draft?');
    expect(req.confirm).toBe('Replace');
    await req.run();
    expect(apply).toHaveBeenCalledTimes(1);
  });
});
