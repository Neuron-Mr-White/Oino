// @vitest-environment jsdom
/**
 * DialogPanel (WP-A9, Z-46): dialogs dock above the composer in **arrival
 * order** with a "1 of N" pager; on the phone the panel collapses to a
 * one-line "Question waiting" bar and the composer keeps working; the
 * approval card answers with `{optionId, feedback?}` and never denies
 * silently when a note is expected.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { UiRequest } from '../../ui/types';
import { ApprovalPanel } from './ApprovalPanel';
import { DialogPanel } from './DialogPanel';

afterEach(() => cleanup());

const select: UiRequest = { id: 'd1', method: 'select', title: 'Which file first?', options: ['math.ts', 'cli.ts'] };
const confirm: UiRequest = { id: 'd2', method: 'confirm', title: 'Overwrite README.md?', message: 'It has 42 lines.' };

describe('DialogPanel — pager', () => {
  it('shows dialogs in arrival order with "1 of N"', () => {
    render(() => <DialogPanel dialogs={[select, confirm]} onAnswer={() => {}} />);
    expect(screen.getByTestId('dialog-count')).toHaveTextContent('1 of 2');
    expect(screen.getByText('Which file first?')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Next dialog'));
    expect(screen.getByTestId('dialog-count')).toHaveTextContent('2 of 2');
    expect(screen.getByText('Overwrite README.md?')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Previous dialog'));
    expect(screen.getByText('Which file first?')).toBeInTheDocument();
  });

  it('answers a select with the option text and a confirm with the boolean', () => {
    const onAnswer = vi.fn();
    render(() => <DialogPanel dialogs={[select, confirm]} onAnswer={onAnswer} />);
    fireEvent.click(screen.getByText('cli.ts'));
    expect(onAnswer).toHaveBeenCalledWith('d1', 'cli.ts');
    fireEvent.click(screen.getByLabelText('Next dialog'));
    fireEvent.click(screen.getByText('Yes'));
    expect(onAnswer).toHaveBeenCalledWith('d2', true);
  });

  it('hides itself when the last dialog is answered', () => {
    const [dialogs, setDialogs] = createSignal<UiRequest[]>([confirm]);
    render(() => <DialogPanel dialogs={dialogs()} onAnswer={() => {}} />);
    expect(screen.getByTestId('dialog-panel')).toBeInTheDocument();
    setDialogs([]);
    expect(screen.queryByTestId('dialog-panel')).toBeNull();
  });

  it('keeps the pager in range when a dialog closes underneath it', () => {
    const [dialogs, setDialogs] = createSignal<UiRequest[]>([select, confirm]);
    render(() => <DialogPanel dialogs={dialogs()} onAnswer={() => {}} />);
    fireEvent.click(screen.getByLabelText('Next dialog'));
    expect(screen.getByTestId('dialog-count')).toHaveTextContent('2 of 2');
    setDialogs([select]);
    expect(screen.getByText('Which file first?')).toBeInTheDocument();
  });
});

describe('DialogPanel — phone collapse (Z-46)', () => {
  it('collapses to a "Question waiting" bar and expands back', () => {
    render(() => <DialogPanel dialogs={[select]} onAnswer={() => {}} phone />);
    expect(screen.getByTestId('dialog-body')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('dialog-collapse'));
    expect(screen.queryByTestId('dialog-body')).toBeNull();
    expect(screen.getByText('Question waiting')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('dialog-collapse'));
    expect(screen.getByTestId('dialog-body')).toBeInTheDocument();
  });

  it('does not offer collapse on the desktop', () => {
    render(() => <DialogPanel dialogs={[select]} onAnswer={() => {}} />);
    expect(screen.queryByTestId('dialog-collapse')).toBeNull();
  });
});

describe('ApprovalPanel (Z-47)', () => {
  const approval: UiRequest = {
    id: 'ap',
    method: 'approval',
    extensionId: 'unipi.workflow',
    toolName: 'bash',
    summary: 'npm test -- --run',
    title: 'Allow bash: npm test -- --run?',
    detail: { t: 'Code', language: 'sh', source: 'npm test -- --run' },
    options: [
      { optionId: 'allow_once', label: 'Allow once', kind: 'allow_once' },
      { optionId: 'allow_always', label: 'Always allow npm test', kind: 'allow_always' },
      { optionId: 'deny', label: 'Deny', kind: 'reject_once' },
      { optionId: 'deny_note', label: 'Deny with note…', kind: 'reject_once' },
    ],
    feedback: { placeholder: 'Why deny? (sent to the agent)' },
  };

  it('shows the summary, the origin badge and the detail; answers with the option id', () => {
    const onAnswer = vi.fn();
    render(() => <ApprovalPanel dialog={approval} onAnswer={onAnswer} />);
    expect(screen.getByTestId('approval-summary')).toHaveTextContent('npm test -- --run');
    expect(screen.getByTestId('approval-origin')).toHaveTextContent('bash');
    expect(screen.getByTestId('approval-detail')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Allow once'));
    expect(onAnswer).toHaveBeenCalledWith({ optionId: 'allow_once' });
  });

  it('"Deny with note" asks for the note first, then sends optionId + feedback', () => {
    const onAnswer = vi.fn();
    render(() => <ApprovalPanel dialog={approval} onAnswer={onAnswer} />);
    fireEvent.click(screen.getByText('Deny with note…'));
    expect(onAnswer).not.toHaveBeenCalled();
    const note = screen.getByLabelText('Note to the agent') as HTMLTextAreaElement;
    fireEvent.input(note, { target: { value: 'use cargo test instead' } });
    fireEvent.click(screen.getByText('Deny with note…'));
    expect(onAnswer).toHaveBeenCalledWith({ optionId: 'deny_note', feedback: 'use cargo test instead' });
  });

  it('the close ✕ cancels (no option)', () => {
    const onAnswer = vi.fn();
    render(() => <ApprovalPanel dialog={approval} onAnswer={onAnswer} />);
    fireEvent.click(screen.getByLabelText('Deny and close'));
    expect(onAnswer).toHaveBeenCalledWith(null);
  });
});
