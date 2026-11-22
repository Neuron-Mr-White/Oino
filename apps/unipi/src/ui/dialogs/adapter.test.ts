// @vitest-environment jsdom
/**
 * The dialog glue (WP-A9): a `UiRequest` becomes the moved card's `Dialog`,
 * and an answer becomes the pi `extension_ui_response` form — `value` for
 * select/input/editor/approval, `confirmed` for confirm, `cancelled` for a
 * dismissal.
 */
import { describe, expect, it } from 'vitest';
import type { UiRequest } from '../types';
import { dialogResponse, piDialog } from './adapter';

describe('piDialog', () => {
  it('maps the pi methods onto the card kinds', () => {
    expect(piDialog({ id: '1', method: 'select', title: 'Pick', options: ['a', 'b'] })).toMatchObject({ kind: 'select', title: 'Pick', options: ['a', 'b'] });
    expect(piDialog({ id: '2', method: 'confirm', title: 'Sure?', message: 'Really' })).toMatchObject({ kind: 'confirm', message: 'Really' });
    expect(piDialog({ id: '3', method: 'input', title: 'Name', placeholder: 'you' })).toMatchObject({ kind: 'input', placeholder: 'you' });
    expect(piDialog({ id: '4', method: 'editor', title: 'Code', prefill: 'x = 1' })).toMatchObject({ kind: 'editor', prefill: 'x = 1' });
  });

  it('takes `{label, description}` options (Oino `optionDetails`) as their labels', () => {
    const d = piDialog({ id: '1', method: 'select', options: [{ label: 'Allow', description: 'once' }, 'Deny'] });
    expect(d.options).toEqual(['Allow', 'Deny']);
  });

  it('maps questions to the ask-user pager and unknown kinds to the "terminal only" card', () => {
    const questions = [{ question: 'Keep going?', options: [{ label: 'Yes' }] }];
    expect(piDialog({ id: '5', method: 'multiSelect', questions })).toMatchObject({ kind: 'ask_user', questions });
    expect(piDialog({ id: '6', method: 'clipboardReadImage' })).toMatchObject({ kind: 'custom' });
  });
});

describe('dialogResponse', () => {
  it('encodes the three pi answer forms', () => {
    expect(dialogResponse('d1', 'cli.ts')).toEqual({ type: 'extension_ui_response', id: 'd1', value: 'cli.ts' });
    expect(dialogResponse('d2', true)).toEqual({ type: 'extension_ui_response', id: 'd2', confirmed: true });
    expect(dialogResponse('d3', null)).toEqual({ type: 'extension_ui_response', id: 'd3', cancelled: true });
    expect(dialogResponse('d4', undefined)).toEqual({ type: 'extension_ui_response', id: 'd4', cancelled: true });
  });

  it('passes an approval answer through as the option payload', () => {
    expect(dialogResponse('ap', { optionId: 'deny_note', feedback: 'no' })).toEqual({
      type: 'extension_ui_response',
      id: 'ap',
      value: { optionId: 'deny_note', feedback: 'no' },
    });
  });
});
