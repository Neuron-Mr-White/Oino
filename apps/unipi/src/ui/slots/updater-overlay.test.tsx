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

it('prefers an installable desktop prompt and skip suppresses the extension', async () => {
  const { DesktopUpdatePrompt } = await import('../../components/UpdateCard');
  const { checkDesktopUpdate, setDesktopUpdateTransport, resetDesktopUpdate } = await import('../../lib/desktopUpdate');
  const { resetUpdateSurface } = await import('../../lib/updateSurface');
  const { fireEvent, waitFor } = await import('@solidjs/testing-library');
  localStorage.clear(); resetDesktopUpdate(); resetUpdateSurface();
  const entry = { id: 'unipi.updater/update', extensionId: 'unipi.updater', scope: 'session', slot: 'overlay', order: 0, seq: 1, rev: 1, title: 'Update available', view: { kind: 'tree', root: { t: 'Layout', children: [{ t: 'Paragraph', text: 'Oino 1.0.2 is available (installed 1.0.1)' }, { t: 'Button', label: 'Skip this version', action: 'skip' }, { t: 'Button', label: 'Later', action: 'close' }] } } } as unknown as UiEntry;
  setDesktopUpdateTransport({ check: async () => ({ current: '1.0.1', version: '1.0.2', kind: 'in_app', downloadPage: 'https://oino.dev' }), install: async () => {} });
  await checkDesktopUpdate(true);
  render(() => <><SlotModal stack={[entry]} target={{ scope: 'session', hostId: 'h', sessionId: 's' }} /><DesktopUpdatePrompt /></>);
  await waitFor(() => expect(screen.getAllByRole('button', { name: 'Skip this version' })).toHaveLength(1));
  expect(screen.getAllByRole('button', { name: 'Later' })).toHaveLength(1);
  fireEvent.click(screen.getByRole('button', { name: 'Skip this version' }));
  await waitFor(() => expect(screen.queryByRole('button', { name: 'Skip this version' })).toBeNull());
  expect(JSON.parse(localStorage.getItem('unipi.desktopUpdate.dismissed.latest')!).version).toBe('1.0.2');
  cleanup(); localStorage.clear(); resetDesktopUpdate(); resetUpdateSurface(); setDesktopUpdateTransport(undefined);
});

it('Escape closes for this session without persisting and restores focus', async () => {
  const { fireEvent, waitFor } = await import('@solidjs/testing-library');
  const { resetUpdateSurface } = await import('../../lib/updateSurface');
  const { resetOverlayDismissals } = await import('./SlotModal');
  localStorage.clear(); resetUpdateSurface(); resetOverlayDismissals();
  const trigger = document.createElement('button'); document.body.append(trigger); trigger.focus();
  const entry = { id: 'unipi.updater/update', extensionId: 'unipi.updater', scope: 'session', slot: 'overlay', order: 0, seq: 9, rev: 1, title: 'Update available', view: { kind: 'tree', root: { t: 'Paragraph', text: 'Oino 1.0.2 is available' } } } as unknown as UiEntry;
  render(() => <SlotModal stack={[entry]} target={{ scope: 'session', hostId: 'h', sessionId: 's' }} />);
  await waitFor(() => expect(document.activeElement?.getAttribute('role')).toBe('dialog'));
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(document.activeElement).toBe(trigger);
  expect(localStorage.getItem('unipi.desktopUpdate.dismissed.latest')).toBeNull();
  trigger.remove(); resetUpdateSurface(); resetOverlayDismissals();
});
