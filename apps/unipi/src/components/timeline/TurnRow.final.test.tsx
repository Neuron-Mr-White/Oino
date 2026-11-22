// @vitest-environment jsdom
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { buildTurns } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import { clearExpansion } from '../../lib/timeline/expansion';
import { TurnRow } from './TurnRow';
import type { TimelineCtx } from './context';

const ctx = {
  session: { state: { byId: {}, entries: [] }, key: 'k' }, hostId: 'h', sessionId: () => 'oin45',
  cwd: () => '/', refs: { hostId: 'h', cwd: () => '/' }, commands: () => [], columnWidth: () => 720,
  readOnly: true,
} as unknown as TimelineCtx;
const text = (text: string) => ({ type: 'text', text });
const thought = { type: 'thinking', thinking: 'Private work' };
const tool = { type: 'toolCall', id: 'tool1', name: 'bash', arguments: { command: 'echo ok' } };
const answer = 'Final answer is visible exactly once.';
function turn(parts: unknown[]) {
  const entries = [
    { type: 'message', id: 'u', parentId: null, message: { role: 'user', content: [text('Question')] } },
    { type: 'message', id: 'a', parentId: 'u', message: { role: 'assistant', content: parts, stopReason: 'stop' } },
  ] as unknown as SessionEntry[];
  return buildTurns({ entries, leafId: 'a', entriesRev: 1, liveTools: {}, run: { running: false } })[0]!;
}
beforeEach(() => { localStorage.clear(); clearExpansion(); });
afterEach(() => { cleanup(); clearExpansion(); });

describe('OIN-45 final answer', () => {
  it.each([
    ['thought tools final', [thought, tool, text(answer)]],
    ['text tools text', [text('Intermediate'), tool, text(answer)]],
    ['final trailing thought', [thought, text(answer), thought]],
    ['final trailing tool', [thought, text(answer), tool]],
    ['long code answer', [thought, tool, text(`${answer}\n\n\`\`\`ts\nconst result = 42;\n\`\`\`\n\nLast paragraph.`)]],
  ])('%s stays visible and unique through the work toggle', (_name, parts) => {
    const r = render(() => <TurnRow turn={turn(parts)} ctx={ctx} />);
    const check = () => {
      const matches = Array.from(r.container.querySelectorAll('.tl-prose')).filter((el) => el.textContent?.includes(answer));
      expect(matches).toHaveLength(1);
      expect(matches[0]!.closest('[aria-hidden="true"]')).toBeNull();
      expect(matches[0]!.closest('.tl-fold')).toBeNull();
    };
    const button = r.container.querySelector<HTMLButtonElement>('.proc-head')!;
    expect(button.getAttribute('aria-expanded')).toBe('false');
    check();
    fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('true');
    check();
    fireEvent.click(button);
    expect(button.getAttribute('aria-expanded')).toBe('false');
    check();
  });
  it('restores expansion after reload without hiding the answer', () => {
    const data = turn([thought, tool, text(answer)]);
    const r = render(() => <TurnRow turn={data} ctx={ctx} />);
    fireEvent.click(r.container.querySelector('.proc-head')!);
    expect(localStorage.getItem('unipi:expansion:oin45|fold:u')).toBe('true');
    r.unmount();
    clearExpansion();
    const reloaded = render(() => <TurnRow turn={data} ctx={ctx} />);
    expect(reloaded.container.querySelector('.proc-head')?.getAttribute('aria-expanded')).toBe('true');
    expect(reloaded.container.querySelector('.tl-prose')?.textContent).toContain(answer);
    expect(reloaded.container.querySelector('.tl-prose')?.closest('[aria-hidden="true"]')).toBeNull();
  });
});
