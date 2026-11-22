// @vitest-environment jsdom
/**
 * DialogCard (UNI-161): ask_user action options fire the action immediately
 * (no "Next" wait), question text renders as markdown, and a `deadline`
 * counts down and auto-dismisses (answers null) once it passes.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DialogCard } from './DialogCard';
import type { Dialog } from '../../lib/chat/store';

afterEach(() => cleanup());

const askUser = (over: Partial<Dialog> = {}): Dialog => ({
  id: 1,
  kind: 'ask_user',
  title: 'Pick one',
  questions: [
    {
      question: 'Keep going **or** stop?',
      header: 'Pick',
      options: [
        { label: 'Keep going', value: 'keep' },
        { label: 'Stop here', value: 'stop', action: 'end_turn' },
        { label: 'Start fresh', value: 'fresh', action: 'new_session', prefill: '/new continue' },
      ],
    },
  ],
  ...over,
});

describe('DialogCard — ask_user', () => {
  it('renders the question text as markdown (bold survives)', () => {
    render(() => <DialogCard dialog={askUser()} onAnswer={() => {}} onHaptic={() => {}} />);
    const q = document.querySelector('.dlg-question');
    expect(q?.innerHTML).toContain('<strong>or</strong>');
  });

  it('picking a plain option waits (single-question: auto-advances to submit)', () => {
    vi.useFakeTimers();
    const onAnswer = vi.fn();
    render(() => <DialogCard dialog={askUser()} onAnswer={onAnswer} onHaptic={() => {}} />);
    fireEvent.click(screen.getByText('Keep going'));
    vi.advanceTimersByTime(200);
    expect(onAnswer).toHaveBeenCalledWith({ type: 'answered', answers: [{ selected: ['keep'], skipped: false }] });
    vi.useRealTimers();
  });

  it('picking an action option (end_turn) submits immediately without waiting for "Next"', () => {
    vi.useFakeTimers();
    const onAnswer = vi.fn();
    render(() => <DialogCard dialog={askUser()} onAnswer={onAnswer} onHaptic={() => {}} />);
    fireEvent.click(screen.getByText('Stop here'));
    vi.advanceTimersByTime(200);
    expect(onAnswer).toHaveBeenCalledWith({ type: 'answered', answers: [{ selected: ['stop'], skipped: false }] });
    vi.useRealTimers();
  });

  it('picking a new_session action option submits its value too (the tool side turns it into the handoff)', () => {
    vi.useFakeTimers();
    const onAnswer = vi.fn();
    render(() => <DialogCard dialog={askUser()} onAnswer={onAnswer} onHaptic={() => {}} />);
    fireEvent.click(screen.getByText('Start fresh'));
    vi.advanceTimersByTime(200);
    expect(onAnswer).toHaveBeenCalledWith({ type: 'answered', answers: [{ selected: ['fresh'], skipped: false }] });
    vi.useRealTimers();
  });
});

describe('DialogCard — deadline countdown', () => {
  it('shows a counting-down mm:ss and auto-dismisses (answers null) once the deadline passes', async () => {
    vi.useFakeTimers();
    const now = Date.now();
    vi.setSystemTime(now);
    const onAnswer = vi.fn();
    const dialog: Dialog = { id: 2, kind: 'confirm', title: 'Sure?', message: 'Really?', deadline: now + 3000 };
    render(() => <DialogCard dialog={dialog} onAnswer={onAnswer} onHaptic={() => {}} />);
    expect(screen.getByText('0:03')).toBeTruthy();
    vi.advanceTimersByTime(1000);
    expect(screen.getByText('0:02')).toBeTruthy();
    vi.advanceTimersByTime(2500);
    expect(onAnswer).toHaveBeenCalledWith(null);
    vi.useRealTimers();
  });

  it('no deadline: no countdown shown', () => {
    render(() => <DialogCard dialog={{ id: 3, kind: 'confirm', title: 'Sure?', message: 'Really?' }} onAnswer={() => {}} onHaptic={() => {}} />);
    expect(document.querySelector('.dlg-countdown')).toBeNull();
  });
});

describe('DialogCard — custom kind', () => {
  it('shows "Answer in the terminal" with no answer controls', () => {
    render(() => <DialogCard dialog={{ id: 4, kind: 'custom', title: "Some other extension's UI" }} onAnswer={() => {}} onHaptic={() => {}} />);
    expect(screen.getByText(/answered in the terminal/i)).toBeTruthy();
  });
});
