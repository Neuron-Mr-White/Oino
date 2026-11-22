import { describe, expect, it } from 'vitest';
import { APP_BINDINGS } from './defaults';
import { hasForegroundSubagent } from './subagents';
import type { LiveTool } from '../session/types';

const tool = (name: string, done?: boolean): LiveTool => ({ id: name, name, args: {}, startedAt: 0, ...(done ? { done } : {}) });

describe('Ctrl+B: foreground subagent or sidebar (P0-10)', () => {
  it('a live run_subagent / read_subagent call is a foreground wait', () => {
    expect(hasForegroundSubagent({ liveTools: { a: tool('run_subagent') } })).toBe(true);
    expect(hasForegroundSubagent({ liveTools: { a: tool('read_subagent') } })).toBe(true);
  });

  it('finished calls, other tools and no session mean the sidebar', () => {
    expect(hasForegroundSubagent({ liveTools: { a: tool('run_subagent', true) } })).toBe(false);
    expect(hasForegroundSubagent({ liveTools: { a: tool('bash') } })).toBe(false);
    expect(hasForegroundSubagent({ liveTools: {} })).toBe(false);
    expect(hasForegroundSubagent(undefined)).toBe(false);
  });

  it('the model cycle is Mod+Alt+P (Mod+P = Files, Alt+P = plan toggle) and the sidebar says it backgrounds', () => {
    const cycle = APP_BINDINGS.find((b) => b.id === 'model.cycle')!;
    expect(cycle.chords).toEqual(['Mod+Alt+P']);
    expect(APP_BINDINGS.filter((b) => b.chords.includes('Mod+Alt+P'))).toHaveLength(1);
    expect(APP_BINDINGS.find((b) => b.id === 'shell.sidebar')!.title).toMatch(/subagent/);
  });
});
