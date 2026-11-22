// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, expect, it, vi } from 'vitest';
import { AutonomySettings } from './AutonomySettings';
import { piSettings, setPiSetting } from '../../settings/hostCmd';
vi.mock('../../settings/hostCmd', () => ({ piSettings: vi.fn(), setPiSetting: vi.fn() }));
vi.mock('../../settings/target', () => ({ settingsHostId: () => 'local' }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
it('loads the global Medium default and saves a validated choice', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(setPiSetting).mockResolvedValue({ ok: true, value: null });
  const view = render(() => <AutonomySettings />);
  const high = await view.findByText('High');
  fireEvent.click(high);
  await waitFor(() => expect(setPiSetting).toHaveBeenCalledWith('autonomy.defaultLevel', 'high', 'global'));
});
it('shows read and write failures without unmounting Settings', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(setPiSetting).mockRejectedValue(new Error('write failed'));
  const view = render(() => <AutonomySettings />);
  fireEvent.click(await view.findByText('High'));
  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('write failed'));
  cleanup();
  vi.mocked(piSettings).mockResolvedValue({ ok: false, error: { code: 'E_TRANSPORT', message: 'offline', retryable: true } });
  const failed = render(() => <AutonomySettings />);
  await waitFor(() => expect(failed.getByRole('alert').textContent).toContain('Could not load autonomy'));
});
