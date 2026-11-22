// @vitest-environment jsdom
/** w7-ui G3-10: the strip does not repeat the mode the header pill already names. */
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, expect, it } from 'vitest';
import { clearStatusScope, isModeSegment, setStatusSegments, StatusStrip } from './StatusStrip';

afterEach(() => { cleanup(); clearStatusScope('h/s'); });

it('recognises a bare mode label and nothing with more to say', () => {
  for (const t of ['Goal', 'swarm', ' Graph ', 'Ralph']) expect(isModeSegment({ text: t })).toBe(true);
  for (const t of ['Goal 2/5', '17 skills ready', 'main ↑1', 'Goals']) expect(isModeSegment({ text: t })).toBe(false);
});

it('renders the other segments and drops the bare mode one (and renders nothing when only that was set)', () => {
  setStatusSegments('h/s', [{ key: 'lh', text: 'Goal' }, { key: 'skills', text: '17 skills ready' }]);
  const r = render(() => <StatusStrip scope="h/s" />);
  const texts = Array.from(r.container.querySelectorAll('.status-seg')).map((e) => e.textContent);
  expect(texts).toEqual(['17 skills ready']);
  setStatusSegments('h/s', [{ key: 'lh', text: 'Swarm' }]);
  expect(r.container.querySelector('[data-testid=status-strip]')).toBeNull();
});
