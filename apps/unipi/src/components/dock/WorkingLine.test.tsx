// @vitest-environment jsdom
/**
 * [owner-fix-2] The working line's clock while the session waits (no host start time): it
 * counts from when the line appeared and keeps counting across host state pushes (each push
 * replaces the run object). Found on the real app rig: "Waiting on … 1s" flipped 1s/2s forever.
 */
import { cleanup, render, screen } from '@solidjs/testing-library';
import { createStore } from 'solid-js/store';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionHandle } from '../../lib/session/store';
import type { RunInfo } from '../../lib/session/types';
import { WorkingLine } from './WorkingLine';

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(1_000_000);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('WorkingLine clock', () => {
  it('keeps counting while host pushes replace the waiting run', async () => {
    const waiting = (): RunInfo => ({ phase: 'waiting', running: false, waitingOn: ['1 subagent'] }) as RunInfo;
    const [state, setState] = createStore({ run: waiting() });
    const session = { state, key: 'h1/s1', hostId: 'h1', sessionId: () => 's1' } as unknown as SessionHandle;
    render(() => <WorkingLine session={session} askCalls={[]} />);
    expect(screen.getByTestId('working-line-text').textContent).toBe('Waiting on 1 subagent');
    for (let i = 0; i < 6; i++) {
      await vi.advanceTimersByTimeAsync(1000);
      setState('run', waiting()); // a session_info push: same state, new object
    }
    await vi.advanceTimersByTimeAsync(1000);
    expect(screen.getByTestId('working-line-time').textContent).toBe('7s');
  });
});
