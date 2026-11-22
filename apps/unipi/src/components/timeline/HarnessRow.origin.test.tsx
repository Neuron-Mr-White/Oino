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

it('N12: the user\'s own command bubble carries no "extension" author chip', () => {
  const turn = { turnId: 't', command: { name: 'swarm', args: 'go', entryId: 'e' }, user: { id: 'u', message: { role: 'user', content: 'go', author: 'extension' } } } as unknown as Turn;
  const view = render(() => <HarnessRow turn={turn} ctx={{} as TimelineCtx} />);
  expect(view.container.textContent).toContain('/swarm');
  expect(view.container.textContent).not.toContain('extension');
  expect(view.container.querySelector('[data-testid=tl-harness-tag]')).toBeNull();
});

it('D9: a /swarm sent with images shows their chips and thumbnails, not "[image 1] [image 2]" text', () => {
  const content = [
    { type: 'text', text: 'compare these two [image 1] [image 2]' },
    { type: 'image', data: 'AAAA', mimeType: 'image/png' },
    { type: 'image', data: 'BBBB', mimeType: 'image/png' },
  ];
  const turn = { turnId: 't', command: { name: 'swarm', args: 'compare these two [image 1] [image 2]', entryId: 'e' }, user: { id: 'u', message: { role: 'user', content, author: 'extension' } } } as unknown as Turn;
  const ctx = { refs: { hostId: 'h', sessionId: () => 's', cwd: () => '/p' } } as unknown as TimelineCtx;
  const view = render(() => <HarnessRow turn={turn} ctx={ctx} />);
  const bubble = view.container.querySelector('.tl-harness-typed')!;
  expect(bubble.textContent).toContain('/swarm compare these two');
  expect(bubble.textContent).not.toContain('[image');
  expect(bubble.querySelectorAll('.ichip[data-kind=image]')).toHaveLength(2);
  expect(view.container.querySelectorAll('.chat-image-row img, .chat-image-row button').length).toBeGreaterThan(0);
});
