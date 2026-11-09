// @vitest-environment jsdom
import { cleanup, render, screen } from '@solidjs/testing-library';
import { afterEach, expect, it } from 'vitest';
import { SlotModal } from './SlotModal';
import type { UiEntry } from '../types';

afterEach(cleanup);
it('does not repeat updater action labels in keyboard hint footer', () => {
  const entry = { id: 'unipi.updater/update', extensionId: 'unipi.updater', scope: 'session', slot: 'overlay', order: 0, seq: 1, rev: 1, title: 'Update available', keys: [{ key: 'n', action: 'skip', label: 'Skip this version' }, { key: 'q', action: 'close', label: 'Later' }], view: { kind: 'tree', root: { t: 'Layout', children: [{ t: 'Button', label: 'Skip this version', action: 'skip' }, { t: 'Button', label: 'Later', action: 'close' }] } } } as unknown as UiEntry;
  render(() => <SlotModal stack={[entry]} target={{ scope: 'session', hostId: 'h', sessionId: 's' }} />);
  expect(screen.getAllByRole('button', { name: 'Skip this version' })).toHaveLength(1);
  expect(screen.getAllByRole('button', { name: 'Later' })).toHaveLength(1);
  expect(screen.queryByLabelText('Panel keys')).toBeNull();
});
