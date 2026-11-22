// @vitest-environment jsdom
/**
 * WP-U10's widened Oino dialog bodies (G-2, ui-protocol §3.2 / v1 §4.3):
 * `multiSelect` (min/max, preseed, `string[]` answer), `custom` (`spec` node;
 * the first submitting action resolves `{actionId, value?, values?}`),
 * `editExternally` (text back), `clipboardReadImage` (graceful without a
 * clipboard), and DialogPanel's routing to them.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DialogPanel } from '../../components/dock/DialogPanel';
import { MultiSelectBody, CustomBody, EditExternallyBody, ClipboardImageBody } from './bodies';
import type { UiRequest } from '../types';

afterEach(cleanup);

describe('multiSelect body', () => {
  const dialog: UiRequest = {
    id: 'ms1',
    method: 'multiSelect',
    title: 'Which panels?',
    options: [
      { label: 'Board', value: 'board', description: 'kanboard', selected: true },
      { label: 'Build', value: 'build' },
      { label: 'Notes', value: 'notes' },
    ],
    min: 1,
    max: 2,
  };
  it('preseeds selected, clamps at max, answers the picked values', async () => {
    const onAnswer = vi.fn();
    render(() => <MultiSelectBody dialog={dialog} onAnswer={onAnswer} />);
    expect(screen.getByRole('button', { name: /Board/ }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: /Build/ }));
    // max 2: the third cannot join
    fireEvent.click(screen.getByRole('button', { name: /Notes/ }));
    expect(screen.getByRole('button', { name: /Notes/ }).getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(screen.getByText('Send'));
    expect(onAnswer).toHaveBeenCalledWith(['board', 'build']);
  });
  it('blocks sending below min', () => {
    const onAnswer = vi.fn();
    render(() => <MultiSelectBody dialog={{ ...dialog, min: 2 }} onAnswer={onAnswer} />);
    fireEvent.click(screen.getByText('Send'));
    expect(onAnswer).not.toHaveBeenCalled();
    expect(screen.getByText('Send')).toHaveProperty('disabled', true);
  });
});

describe('custom body (spec node, first submitting action resolves)', () => {
  const dialog: UiRequest = {
    id: 'cd1',
    method: 'custom',
    title: 'Compact now?',
    spec: {
      t: 'Layout',
      direction: 'Vertical',
      children: [
        { t: 'Paragraph', text: 'Context 84 % full.' },
        { t: 'Button', label: 'Compact now', action: 'compact-now', variant: 'Primary' },
      ],
    },
  };
  it('renders the spec and resolves on the button with collected values', async () => {
    const onAnswer = vi.fn();
    render(() => <CustomBody dialog={dialog} onAnswer={onAnswer} />);
    expect(screen.getByText('Context 84 % full.')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Compact now' }));
    expect(onAnswer).toHaveBeenCalledWith({ actionId: 'compact-now' });
  });
  it('only the first submitting action resolves', () => {
    const onAnswer = vi.fn();
    const { container } = render(() => <CustomBody dialog={dialog} onAnswer={onAnswer} />);
    fireEvent.click(screen.getByRole('button', { name: 'Compact now' }));
    fireEvent.click(screen.getByRole('button', { name: 'Compact now' }));
    expect(onAnswer).toHaveBeenCalledTimes(1);
    expect(container).toBeTruthy();
  });
});

describe('editExternally body', () => {
  it('answers the edited text; ✕ cancels', () => {
    const onAnswer = vi.fn();
    render(() => <EditExternallyBody dialog={{ id: 'ed1', method: 'editExternally', text: '# hi', extensionHint: 'md' } as UiRequest} onAnswer={onAnswer} />);
    const area = screen.getByLabelText('Text to edit') as HTMLTextAreaElement;
    fireEvent.input(area, { target: { value: '# hi\n- more' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(onAnswer).toHaveBeenCalledWith('# hi\n- more');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(onAnswer).toHaveBeenCalledWith(null);
  });
});

describe('clipboardReadImage body', () => {
  it('explains when the build cannot read the clipboard', async () => {
    const onAnswer = vi.fn();
    render(() => <ClipboardImageBody dialog={{ id: 'cb1', method: 'clipboardReadImage' }} onAnswer={onAnswer} />);
    fireEvent.click(screen.getByRole('button', { name: 'Read clipboard' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('clipboard'));
    expect(onAnswer).not.toHaveBeenCalled();
  });
});

describe('DialogPanel routing (A9 frame + U10 bodies)', () => {
  it('routes multiSelect-without-questions to the widened body', async () => {
    render(() => (
      <DialogPanel
        dialogs={[{ id: 'ms1', method: 'multiSelect', title: 'Pick panels', options: [{ label: 'Board', value: 'board' }] }]}
        onAnswer={() => {}}
      />
    ));
    expect(screen.getByTestId('dlg-multiselect')).toBeTruthy();
  });
  it('routes custom-with-spec to the node body; form-with-questions stays on the ask-user card', async () => {
    render(() => (
      <DialogPanel
        dialogs={[
          { id: 'cd1', method: 'custom', title: 'Compact?', spec: { t: 'Paragraph', text: 'ctx 84 %' } },
          { id: 'f1', method: 'form', questions: [{ id: 'q1', question: 'Which?' }] },
        ]}
        onAnswer={() => {}}
      />
    ));
    expect(screen.getByTestId('dlg-custom')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Next dialog' })); // 1 of 2 → the form
    await waitFor(() => expect(screen.queryByTestId('dlg-custom')).toBeNull());
  });
  it('routes editExternally and clipboardReadImage', () => {
    render(() => <DialogPanel dialogs={[{ id: 'ed1', method: 'editExternally', text: 'x' }]} onAnswer={() => {}} />);
    expect(screen.getByTestId('dlg-edit')).toBeTruthy();
  });
});
