// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import SearchSettings from './SearchSettings';
import { piSettings, setPiSetting } from '../../settings/hostCmd';
import { requestParsed } from '../../lib/host';
vi.mock('../../settings/hostCmd', () => ({ piSettings: vi.fn(), setPiSetting: vi.fn() }));
vi.mock('../../settings/target', () => ({ settingsHostId: () => 'local', settingsProjectCwd: () => '/project' }));
vi.mock('../../lib/host', () => ({ hostClient: () => ({}), requestParsed: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
beforeEach(() => {
  vi.mocked(requestParsed).mockResolvedValue({ ok: true, value: { semanticSearch: true } });
});
it('defaults off and explains session snapshot semantics', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(setPiSetting).mockResolvedValue({ ok: true, value: null });
  const view = render(() => <SearchSettings />);
  const off = await view.findByRole('radio', { name: 'Off' });
  expect(off.getAttribute('aria-checked')).toBe('true');
  expect(view.getByText(/Changes apply to new sessions/)).toBeTruthy();
  fireEvent.click(view.getByRole('radio', { name: 'On' }));
  await waitFor(() => expect(setPiSetting).toHaveBeenCalledWith('search.semanticEnabled', true, 'global'));
  expect(vi.mocked(requestParsed).mock.calls.every((call) => (call[2] as { cmd: { kind: string } }).cmd.kind === 'search_capabilities')).toBe(true);
});
it('requires explicit download and project index consent', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(requestParsed).mockImplementation(async (_client, _op, args) => ({ ok: true, value: (args as { cmd: { kind: string } }).cmd.kind === 'search_capabilities' ? { semanticSearch: true } : { success: true } }) as never);
  const view = render(() => <SearchSettings />);
  const button = await view.findByRole('button', { name: 'Build index' });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  expect(vi.mocked(requestParsed).mock.calls.every((call) => (call[2] as { cmd: { kind: string } }).cmd.kind === 'search_capabilities')).toBe(true);
  fireEvent.click(view.getByRole('checkbox'));
  fireEvent.click(button);
  await waitFor(() => expect(requestParsed).toHaveBeenCalledWith(expect.anything(), 'host_cmd', { cmd: { kind: 'build_search_index', cwd: '/project', consent: true } }, expect.anything()));
  await waitFor(() => expect(view.getByRole('status').textContent).toContain('Index ready'));
  expect((button as HTMLButtonElement).disabled).toBe(true);
});
it('keeps build errors visible', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(requestParsed).mockImplementation(async (_client, _op, args) => ((args as { cmd: { kind: string } }).cmd.kind === 'search_capabilities' ? { ok: true, value: { semanticSearch: true } } : { ok: false, error: { code: 'E_UNSUPPORTED', message: 'CPU engine not included', retryable: false } }) as never);
  const view = render(() => <SearchSettings />);
  await view.findByRole('button', { name: 'Build index' });
  fireEvent.click(view.getByRole('checkbox'));
  fireEvent.click(view.getByRole('button', { name: 'Build index' }));
  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('CPU engine not included'));
});

it('shows unavailable state when the running host has no asset', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: { search: { semanticEnabled: true } } });
  vi.mocked(requestParsed).mockResolvedValue({ ok: true, value: { semanticSearch: false } });
  const view = render(() => <SearchSettings />);
  await view.findByText(/not available for this build/);
  expect(view.queryByRole('radiogroup', { name: 'Semantic search' })).toBeNull();
  expect(view.queryByRole('button', { name: 'Build index' })).toBeNull();
  expect(view.queryByRole('checkbox')).toBeNull();
  expect(setPiSetting).not.toHaveBeenCalled();
});
it('offers a consented install when an asset is available', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(requestParsed).mockImplementation(async (_client, _op, args) => ({ ok: true, value: (args as { cmd: { kind: string } }).cmd.kind === 'search_capabilities' ? { semanticSearch: false, available: true, installed: false, size: 85000000 } : { success: true } }) as never);
  const view = render(() => <SearchSettings />);
  const button = await view.findByRole('button', { name: 'Install semantic search' });
  expect((button as HTMLButtonElement).disabled).toBe(true);
  expect(view.getByText(/85.0 MB/)).toBeTruthy();
  fireEvent.click(view.getByRole('checkbox'));
  fireEvent.click(button);
  await waitFor(() => expect(requestParsed).toHaveBeenCalledWith(expect.anything(), 'host_cmd', { cmd: { kind: 'install_search', consent: true } }, expect.anything()));
});
it('fails closed for older or unreachable hosts', async () => {
  vi.mocked(piSettings).mockResolvedValue({ ok: true, value: {} });
  vi.mocked(requestParsed).mockResolvedValue({ ok: false, error: { code: 'E_UNSUPPORTED', message: 'unknown command', retryable: false } });
  const view = render(() => <SearchSettings />);
  await view.findByText(/not available for this build/);
  expect(view.queryByRole('radiogroup')).toBeNull();
  expect(view.queryByRole('button', { name: 'Build index' })).toBeNull();
});
