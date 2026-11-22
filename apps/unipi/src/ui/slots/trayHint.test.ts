import { describe, expect, it } from 'vitest';
import { liveTrayEntries, liveTrayLine, trayButtonText } from './trayHint';
import { sanitizeTrayBody } from '../../components/session/RunningTab';
import type { UiEntry } from '../types';

describe('E-17 / G-08 live tray hint & button', () => {
  it('drops finished work and keeps what runs (both lines and Paragraph tree views)', () => {
    expect(liveTrayLine('Background tasks 4 · Subagents 2 · ↓ open')).toBeNull();
    expect(liveTrayLine('Background tasks 4 · Subagents 2 running · ↓ open')).toBe('Subagents 2 running · ↓ open');
    expect(liveTrayLine('Subagents 3 (1 running) · ↓ open')).toBe('Subagents 1 running · ↓ open');
    expect(liveTrayLine('Bg 4/2 · ↓ open')).toBe('Bg 2 running · ↓ open');
    expect(liveTrayLine('something else')).toBeUndefined();

    const makeTree = (text: string): UiEntry =>
      ({
        id: 'widget:work-tray-strip',
        extensionId: 'unipi.core',
        scope: 'session',
        slot: 'widgetBelow',
        order: 0,
        seq: 1,
        rev: 1,
        view: { kind: 'tree', root: { t: 'Paragraph', text } as never },
      }) as UiEntry;

    expect(liveTrayEntries([makeTree('Background tasks 2 · Subagents 2 · ↓ open')])).toEqual([]);
    const live = makeTree('Background tasks 2 (1 running) · ↓ open');
    expect(liveTrayEntries([live])).toHaveLength(1);
    expect(trayButtonText(live)).toBe('Background tasks 1 running');
  });

  it('sanitizes TUI KeyHints and disables Stop all when idle (G-08)', () => {
    const body = {
      t: 'Layout',
      children: [
        { t: 'Button', label: 'Stop all', onPress: 'stopall', tone: 'warning' },
        { t: 'Button', label: 'Dismiss finished', onPress: 'dismissall' },
        { t: 'KeyHints', hints: [{ keys: 's', label: 'stop' }, { keys: 'esc', label: 'close' }] },
      ],
    };
    const idle = sanitizeTrayBody(body, false) as { children: Array<Record<string, unknown>> };
    expect(idle.children).toHaveLength(2);
    expect(idle.children[0]).toMatchObject({ label: 'Stop all', disabled: true });
    expect(idle.children[0]!.tone).toBeUndefined();
    const active = sanitizeTrayBody(body, true) as { children: Array<Record<string, unknown>> };
    expect(active.children).toHaveLength(2);
    expect(active.children[0]!.disabled).toBeUndefined();
  });
});

import { humanStatus } from './statusFeed';
describe('F-13 human status', () => {
  it('humanises and hides', () => {
    expect(humanStatus('🧠 0 mem')).toBeNull();
    expect(humanStatus('🧠 3 mem')).toBe('3 memories');
    expect(humanStatus('skills: 17/17 listed')).toBe('17 skills ready');
    expect(humanStatus('Graph')).toBeNull();
    expect(humanStatus('main ↑1')).toBe('main ↑1');
  });
});
