// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { createResource } from 'solid-js';
import { afterEach, expect, it, vi } from 'vitest';
import { PhoneSettingsRoot } from './PhoneSettings';
import { createMockHost } from '../../lib/host/mock';
import { setHostClientFactory } from '../../lib/host';
import { piSettings } from '../../settings/hostCmd';
import { selectSettingsHost, settingsHostId } from '../../settings/target';

vi.mock('../../hosts/context', () => ({ useHostsOptional: () => ({ hosts: () => [
  { id: 'local', name: 'Phone', transport: 'local' },
  { id: 'pc', name: 'oi', transport: 'iroh' },
] }) }));
vi.mock('../../components/phone', () => ({ ListGroup: (props: { children: unknown }) => props.children, ListRow: () => null, RowIcon: () => null }));
vi.mock('../../components/ConnectionState', () => ({ hostConnState: () => ({ state: 'connected' }) }));
vi.mock('../../components/shell/Rail', () => ({ railItems: () => [] }));

afterEach(() => { cleanup(); setHostClientFactory(undefined); selectSettingsHost('local'); });

it('switches the settings target and displayed host values in both directions', async () => {
  const world = createMockHost({ name: 'selector', hosts: [
    { id: 'local', name: 'Phone', settings: { pi: { defaultModel: 'phone-model' } } },
    { id: 'pc', name: 'oi', settings: { pi: { defaultModel: 'pc-model' } } },
  ] });
  setHostClientFactory(world.client);
  selectSettingsHost('pc');
  function Values() {
    const [values] = createResource(settingsHostId, async (host) => {
      const result = await piSettings(host);
      return result.ok ? String(result.value.defaultModel) : 'error';
    });
    return <output data-testid="model-value">{values()}</output>;
  }
  const view = render(() => <><PhoneSettingsRoot search={() => null} /><Values /></>);
  await waitFor(() => expect(view.getByTestId('model-value').textContent).toBe('pc-model'));
  const selector = view.getByTestId('settings-host-selector') as HTMLSelectElement;
  expect(selector.value).toBe('pc');
  fireEvent.change(selector, { target: { value: 'local' } });
  await waitFor(() => expect(view.getByTestId('model-value').textContent).toBe('phone-model'));
  fireEvent.change(selector, { target: { value: 'pc' } });
  await waitFor(() => expect(view.getByTestId('model-value').textContent).toBe('pc-model'));
  expect(world.log.filter((entry) => entry.op === 'host_cmd').map((entry) => entry.host)).toEqual(['pc', 'local', 'pc']);
});
