import { describe, expect, it, vi } from 'vitest';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
import { dashboardPane, parseInfoGroups } from './InfoTab';
import type { UiEntry } from '../../ui/types';

describe('P1-03: Info data from the extensions', () => {
  it('decodes an info-screen.groups card with its declared stats (raw skipped, empty dropped)', () => {
    const reply = {
      id: 'mcp',
      name: 'MCP Servers',
      config: { stats: [{ id: 'total', label: 'Total servers' }, { id: 'failed', label: 'Failed' }, { id: 'hidden', label: 'H', show: false }] },
      data: { raw: { value: '' }, total: { value: '2' }, failed: { value: '1', detail: 'github' }, hidden: { value: 'x' } },
    };
    expect(parseInfoGroups('unipi.mcp', reply)).toEqual([
      { owner: 'unipi.mcp', id: 'mcp', name: 'MCP Servers', stats: [{ id: 'total', label: 'Total servers', value: '2' }, { id: 'failed', label: 'Failed', value: '1', detail: 'github' }] },
    ]);
    expect(parseInfoGroups('x', null)).toEqual([]);
    expect(parseInfoGroups('x', [{ id: 'a', data: { n: { value: 3 } } }])[0]?.stats).toEqual([{ id: 'n', label: 'n', value: '3' }]);
  });

  it('picks one page of the dashboard fallback Tabs tree; an unloaded spinner is nothing', () => {
    const entry = {
      id: 'unipi.info-screen/dashboard',
      view: { kind: 'reserved', component: 'sessionInfo', fallback: { t: 'Tabs', titles: [], panes: { usage: { t: 'Paragraph', text: 'ok' }, tools: { t: 'Spinner' } } } },
    } as unknown as UiEntry;
    expect(dashboardPane(entry, 'usage')).toEqual({ t: 'Paragraph', text: 'ok' });
    expect(dashboardPane(entry, 'tools')).toBeUndefined();
    expect(dashboardPane(undefined, 'usage')).toBeUndefined();
  });
});
