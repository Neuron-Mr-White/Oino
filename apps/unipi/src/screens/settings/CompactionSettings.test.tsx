import { render, screen, fireEvent, waitFor, cleanup } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
const mocks = vi.hoisted(() => ({ load: vi.fn(), save: vi.fn() }));
vi.mock('../../settings/hostCmd', () => ({ piSettings: mocks.load, setPiSetting: mocks.save }));
vi.mock('../../settings/target', () => ({ settingsHostId: () => 'test' }));
import CompactionSettings from './CompactionSettings';
afterEach(cleanup);
beforeEach(() => {
  mocks.load.mockResolvedValue({ ok: true, value: { compaction: { tokenLimit: 12000 } } });
  mocks.save.mockReset(); mocks.save.mockResolvedValue({ ok: true, value: null });
});
describe('CompactionSettings', () => {
  it('validates positive token counts and saves the allowed keys', async () => {
    render(() => <CompactionSettings />);
    const input = await screen.findByLabelText('Input token limit');
    fireEvent.change(input, { target: { value: '0' } });
    expect(await screen.findByRole('alert')).toHaveTextContent('positive whole token count');
    expect(mocks.save).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: '25000' } });
    await waitFor(() => expect(mocks.save).toHaveBeenCalledWith('compaction.tokenLimit', 25000, 'global'));
    await waitFor(() => expect(screen.getByLabelText('Input token limit')).not.toBeDisabled());
    fireEvent.change(screen.getByLabelText('Model token limits'), { target: { value: '{"gpt":24000}' } });
    await waitFor(() => expect(mocks.save).toHaveBeenCalledWith('compaction.tokenLimitPerModel', { gpt: 24000 }, 'global'));
  });
  it('surfaces save errors', async () => {
    mocks.save.mockResolvedValue({ ok: false, error: { message: 'disk unavailable' } });
    render(() => <CompactionSettings />);
    fireEvent.change(await screen.findByLabelText('Input token limit'), { target: { value: '42' } });
    expect(await screen.findByRole('alert')).toHaveTextContent('disk unavailable');
  });
});
