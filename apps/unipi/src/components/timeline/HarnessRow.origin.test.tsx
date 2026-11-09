// @vitest-environment jsdom
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { HarnessRow } from './HarnessRow';
import type { TimelineCtx } from './context';
import type { Turn } from '../../lib/session/turns';

afterEach(cleanup);
describe('host notification row', () => {
  for (const [source, title] of [['Long-horizon', 'Graph checkpoint'], ['System', 'Loop guard'], ['Memory', 'Reminder'], ['Host', 'Wake']]) {
    it(`renders ${source} without a user bubble or steer`, () => {
      const turn = { turnId: 't', user: { id: 'u', message: { role: 'user', content: 'Notification', details: { oinoProvenance: { source, title } } } } } as unknown as Turn;
      const view = render(() => <HarnessRow turn={turn} ctx={{} as TimelineCtx} />);
      expect(view.container.textContent).toContain(source);
      expect(view.container.textContent).toContain('Notification');
      expect(view.container.querySelector('.chat-bubble, [data-steer]')).toBeNull();
    });
  }
});

it('labels attributed notifications by text and displays repeat count', () => {
  for (const [text, label] of [['Agent graph checkpoint: x', 'Graph'], ['Loop warning', 'System'], ['Memory reminder', 'Memory'], ['Host wake', 'Wake']]) {
    const turn = { turnId: text, harnessCount: 3, user: { id: text, message: { role: 'user', content: text, author: 'extension' } } } as unknown as Turn;
    const view = render(() => <HarnessRow turn={turn} ctx={{} as TimelineCtx} />);
    expect(view.container.textContent).toContain(label);
    expect(view.container.textContent).toContain('×3');
    view.unmount();
  }
});
