// @vitest-environment jsdom
import { render, cleanup, screen } from '@solidjs/testing-library';
import { createSignal, onCleanup, onMount } from 'solid-js';
import { afterEach, describe, expect, it } from 'vitest';
import { registerPaneKind, type PaneTabProps } from '../../lib/panes/kinds';
import type { PaneTab } from '../../lib/panes';
import { PaneBody } from './PaneBody';

afterEach(cleanup);

describe('PaneBody', () => {
  it('a retitled tab keeps its mounted view (keyed by id), and sees the new title', () => {
    let mounts = 0;
    let unmounts = 0;
    const View = (p: PaneTabProps) => {
      onMount(() => mounts++);
      onCleanup(() => unmounts++);
      return <p data-testid="view">{p.tab.title ?? 'none'}</p>;
    };
    const off = registerPaneKind({ kind: 'probe-kind', icon: () => null, title: () => 'Probe', singleton: false, scope: 'project', component: View });
    const [tabs, setTabs] = createSignal<PaneTab[]>([{ id: 't1', kind: 'probe-kind' }]);
    render(() => <PaneBody area="side" scope={{ hostId: 'h', projectKey: '/p' }} tabs={tabs()} activeId="t1" onClose={() => {}} onTitle={() => {}} />);
    expect(mounts).toBe(1);
    setTabs([{ id: 't1', kind: 'probe-kind', title: 'Page A' }]);
    setTabs([{ id: 't1', kind: 'probe-kind', title: 'Page B' }]);
    expect(mounts).toBe(1);
    expect(unmounts).toBe(0);
    expect(screen.getByTestId('view').textContent).toBe('Page B');
    setTabs([]);
    expect(unmounts).toBe(1);
    off();
  });
});
