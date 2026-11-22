// @vitest-environment jsdom
/** w6-ui N3: one compact, collapsible command note per chat. */
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { noteCommandRun, resetReports, routeNotify, showReportsInTimeline } from '../../lib/reports';
import { CommandReports } from './CommandReports';

afterEach(() => {
  cleanup();
  resetReports();
});

describe('CommandReports', () => {
  it('three busy-graph notes show as one row; a long note starts folded to its headline and unfolds on click', () => {
    const off = showReportsInTimeline('h/s');
    const view = render(() => <CommandReports sessionKey="h/s" />);
    for (let i = 0; i < 3; i++) {
      noteCommandRun('h/s', 'swarm x', 1000 + i);
      routeNotify(`Swarm not started: a graph is already running in this chat.\nFinish or stop it first.\nrun ${i}`, 'info', 'h/s', 1500 + i);
    }
    expect(view.container.querySelectorAll('[data-testid=tl-command-report]')).toHaveLength(1);
    expect(view.container.querySelector('.tl-report-body')).toBeNull();
    expect(view.container.querySelector('.tl-report-peek')?.textContent).toContain('Swarm not started');
    fireEvent.click(view.getByTestId('tl-report-fold'));
    expect(view.container.querySelector('.tl-report-body')?.textContent).toContain('run 2');
    off();
  });

  it('a short one-line note is open', () => {
    const off = showReportsInTimeline('h/s');
    const view = render(() => <CommandReports sessionKey="h/s" />);
    noteCommandRun('h/s', 'doctor', 1000);
    routeNotify('x'.repeat(170), 'info', 'h/s', 1500);
    expect(view.container.querySelector('.tl-report-body')).not.toBeNull();
    off();
  });
});
