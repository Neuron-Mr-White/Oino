// @vitest-environment jsdom
/** The model row's modality icons: input icons from the catalogue, outputs only when non-text. */
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@solidjs/testing-library';
import { ModalityIcons, modalities, modalityOf, rowModalities } from './ModalityIcons';

afterEach(cleanup);

describe('modality icons', () => {
  it('maps catalogue names, dedupes and orders them', () => {
    expect(modalityOf('PDF')).toBe('file');
    expect(modalityOf('speech')).toBe('audio');
    expect(modalityOf('embedding')).toBeUndefined();
    expect(modalities(['image', 'text', 'pdf', 'file', 'video', 'audio', 'x'])).toEqual(['text', 'image', 'audio', 'video', 'file']);
  });

  it('shows outputs only when the model makes something besides text', () => {
    expect(rowModalities({ input: ['text'], output: ['text'] }).output).toEqual([]);
    expect(rowModalities({ input: ['text'], output: ['text', 'image'] }).output).toEqual(['text', 'image']);
    expect(rowModalities({ input: ['text'] }).output).toEqual([]);
  });

  it('renders one labelled icon per input modality', () => {
    render(() => <ModalityIcons m={{ input: ['text', 'image', 'audio', 'video', 'file'] }} />);
    for (const label of ['Accepts text', 'Accepts images', 'Accepts audio', 'Accepts video', 'Accepts files (PDF)']) {
      const el = screen.getByLabelText(label);
      expect(el.getAttribute('title')).toBe(label);
      expect(el.querySelector('svg')).toBeTruthy();
    }
    expect(screen.getByTestId('model-modalities').textContent).toBe('');
    expect(screen.queryByLabelText('Generates images')).toBeNull();
  });

  it('adds an arrow and the output icons for an image / speech model', () => {
    render(() => <ModalityIcons m={{ input: ['text'], output: ['image', 'audio'] }} />);
    expect(screen.getByLabelText('Generates images')).toBeTruthy();
    expect(screen.getByLabelText('Generates audio')).toBeTruthy();
    expect(screen.getByTestId('model-modalities').querySelector('.mp-mod-arrow')).toBeTruthy();
  });

  it('renders nothing without metadata', () => {
    render(() => <ModalityIcons m={{}} />);
    expect(screen.queryByTestId('model-modalities')).toBeNull();
  });
});
