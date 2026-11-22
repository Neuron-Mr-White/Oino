import { afterEach, expect, it } from 'vitest';
import { createMockHost } from '../../lib/host/mock';
import { setHostClientFactory } from '../../lib/host';
import { piSettings, setPiSetting } from '../hostCmd';
import { selectSettingsHost } from '../target';

afterEach(() => { setHostClientFactory(undefined); selectSettingsHost('local'); });

it('routes settings reads and writes to the selected host', async () => {
  const world = createMockHost({ name: 'settings-routing', hosts: [
    { id: 'local', name: 'Phone', settings: { pi: { defaultModel: 'phone' } } },
    { id: 'pc', name: 'PC', settings: { pi: { defaultModel: 'pc' } } },
  ] });
  setHostClientFactory(world.client);
  selectSettingsHost('pc');
  expect(await piSettings()).toMatchObject({ ok: true, value: { defaultModel: 'pc' } });
  expect((await setPiSetting('defaultModel', 'new')).ok).toBe(true);
  expect(await piSettings()).toMatchObject({ ok: true, value: { defaultModel: 'new' } });
  selectSettingsHost('local');
  expect(await piSettings()).toMatchObject({ ok: true, value: { defaultModel: 'phone' } });
  expect(world.log.map((entry) => entry.host)).toEqual(['pc', 'pc', 'pc', 'local']);
});
