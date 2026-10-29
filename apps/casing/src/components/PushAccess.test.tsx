// @vitest-environment jsdom
/** UNI-248: Settings → Files from your PCs — on / off (with Ask) / unreachable per PC. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import type { JSX } from 'solid-js';
import { PushAccess } from './PushAccess';
import { setPushTransport, type PushTransport } from '../lib/push';
import { setPlatformOverride } from '../lib/platform';
import { setSharingTransport } from '../lib/sharing';

const Section = (p: { title: string; icon: JSX.Element; children: JSX.Element }) => <section data-title={p.title}>{p.children}</section>;

afterEach(() => {
  cleanup();
  setPushTransport(undefined);
  setSharingTransport(undefined);
  setPlatformOverride(undefined);
});

describe('Files from your PCs', () => {
  it('per PC: on, off with Turn on → Ask card, unreachable', async () => {
    setPlatformOverride('tauri-mobile');
    setSharingTransport({
      localHost: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
      status: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
      change: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
      leave: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
      ask: async () => ({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'x', retryable: false } }),
      requests: async () => ({ ok: true, value: [] }),
    });
    setPushTransport({
      list: async (hostId: string) => {
        if (hostId === 'down') throw new Error('no route');
        return { enabled: hostId === 'on', host_name: hostId, pushes: [] };
      },
    } as unknown as PushTransport);
    render(() => <PushAccess hosts={[{ id: 'on', name: 'coffee' }, { id: 'off', name: 'laptop' }, { id: 'down', name: 'nas' }]} Section={Section} />);
    await waitFor(() => expect(screen.getByTestId('push-access-on').dataset.state).toBe('on'));
    await waitFor(() => expect(screen.getByTestId('push-access-off').dataset.state).toBe('off'));
    await waitFor(() => expect(screen.getByTestId('push-access-down').dataset.state).toBe('offline'));
    expect(screen.getByTestId('push-access-off').textContent).toContain('Off');
    fireEvent.click(screen.getByText('Turn on'));
    expect((await screen.findByTestId('share-ask-push')).textContent).toContain('Sending files to phones is off');
  });

  it('nothing when no PC is paired', () => {
    const { container } = render(() => <PushAccess hosts={[]} Section={Section} />);
    expect(container.textContent).toBe('');
  });
});
