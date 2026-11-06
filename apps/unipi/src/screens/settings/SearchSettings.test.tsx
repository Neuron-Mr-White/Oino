// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, expect, it, vi } from 'vitest';
import SearchSettings from './SearchSettings';
import { piSettings, setPiSetting } from '../../settings/hostCmd';
import { requestParsed } from '../../lib/host';
vi.mock('../../settings/hostCmd', () => ({ piSettings: vi.fn(), setPiSetting: vi.fn() }));
vi.mock('../../settings/target', () => ({ settingsHostId: () => 'local', settingsProjectCwd: () => '/project' }));
vi.mock('../../lib/host', () => ({ hostClient: () => ({}), requestParsed: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
it('defaults off and explains session snapshot semantics', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(setPiSetting).mockResolvedValue({ ok: true, value: null });
  const view = render(() => <SearchSettings />);
  const off = await view.findByRole('radio', { name: 'Off' });
  expect(off.getAttribute('aria-checked')).toBe('true');
  expect(view.getByText(/Changes apply to new sessions/)).toBeTruthy();
  fireEvent.click(view.getByRole('radio', { name: 'On' }));
  await waitFor(() => expect(setPiSetting).toHaveBeenCalledWith('search.semanticEnabled', true, 'global'));
  expect(requestParsed).not.toHaveBeenCalled();
});
it('requires explicit download and project index consent', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(requestParsed).mockResolvedValue({ ok: true, value: { success: true } });
  const view = render(() => <SearchSettings />);
  const button = await view.findByRole('button', { name: 'Build index' });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  expect(requestParsed).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('checkbox'));
  fireEvent.click(button);
  await waitFor(() => expect(requestParsed).toHaveBeenCalledWith(expect.anything(), 'host_cmd', { cmd: { kind: 'build_search_index', cwd: '/project', consent: true } }, expect.anything()));
  await waitFor(() => expect(view.getByRole('status').textContent).toContain('Index ready'));
  expect((button as HTMLButtonElement).disabled).toBe(true);
});
it('keeps build errors visible', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(requestParsed).mockResolvedValue({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'CPU engine not included', retryable: false } });
  const view = render(() => <SearchSettings />);
  await view.findByRole('button', { name: 'Build index' });
  fireEvent.click(view.getByRole('checkbox'));
  fireEvent.click(view.getByRole('button', { name: 'Build index' }));
  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('CPU engine not included'));
});
