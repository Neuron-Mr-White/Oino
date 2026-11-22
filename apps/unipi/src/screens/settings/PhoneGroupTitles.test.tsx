// @vitest-environment jsdom
/** w6-ui N11: at phone width the desktop app listed two groups both headed "This computer". */
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, expect, it, vi } from 'vitest';
import { PhoneSettingsRoot } from './PhoneSettings';
import { setPlatformOverride } from '../../lib/platform';

vi.mock('../../hosts/context', () => ({ useHostsOptional: () => ({ hosts: () => [{ id: 'local', name: 'Box', transport: 'local' }] }) }));
vi.mock('../../components/phone', () => ({
  ListGroup: (props: { title?: string; children: unknown }) => <section><h3 data-testid="group-title">{props.title}</h3>{props.children as never}</section>,
  ListRow: () => null,
  RowIcon: () => null,
}));
vi.mock('../../components/ConnectionState', () => ({ hostConnState: () => ({ state: 'connected' }) }));
vi.mock('../../components/shell/Rail', () => ({ railItems: () => [] }));
afterEach(() => { cleanup(); setPlatformOverride(undefined); });

const titles = (v: ReturnType<typeof render>) => v.queryAllByTestId('group-title').map((e) => e.textContent).filter(Boolean);

it('the desktop app at phone width has distinct group headings', () => {
  setPlatformOverride('tauri-desktop');
  const t = titles(render(() => <PhoneSettingsRoot search={() => null} />));
  expect(new Set(t).size).toBe(t.length);
  expect(t).toContain('This app');
  expect(t).toContain('Desktop only');
  expect(t.filter((x) => /^This computer/.test(x ?? ''))).toHaveLength(1);
});

it('a phone still says "This phone"', () => {
  setPlatformOverride('tauri-mobile');
  expect(titles(render(() => <PhoneSettingsRoot search={() => null} />))).toContain('This phone');
});
