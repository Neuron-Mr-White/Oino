import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { createFileDrop, isSubmitChord } from './fileDrop';
import { setPlatformOverride } from '../platform';

function dragEvent(type: string, types: string[], files: File[] = []) {
  const event = new Event(type, { cancelable: true }) as DragEvent;
  Object.defineProperty(event, 'dataTransfer', { value: { types, files, dropEffect: 'none' } });
  Object.defineProperty(event, 'currentTarget', { value: document.body });
  return event;
}

describe('file drop', () => {
  afterEach(() => setPlatformOverride(undefined));
  it('accepts OS files on desktop/web, ignores card drags', () => {
    setPlatformOverride('web');
    const onFiles = vi.fn();
    createRoot((dispose) => {
      const drop = createFileDrop(onFiles);
      const over = dragEvent('dragover', ['Files']);
      drop.handlers.onDragOver(over);
      expect(over.defaultPrevented).toBe(true);
      expect(drop.over()).toBe(true);
      const file = new File(['x'], 'a.png');
      drop.handlers.onDrop(dragEvent('drop', ['Files'], [file]));
      expect(onFiles).toHaveBeenCalledWith([file]);
      expect(drop.over()).toBe(false);
      const card = dragEvent('dragover', ['application/x-unipi-task']);
      drop.handlers.onDragOver(card);
      expect(card.defaultPrevented).toBe(false);
      dispose();
    });
  });
  it('is off on the phone', () => {
    setPlatformOverride('tauri-mobile');
    const onFiles = vi.fn();
    createRoot((dispose) => {
      const drop = createFileDrop(onFiles);
      drop.handlers.onDrop(dragEvent('drop', ['Files'], [new File(['x'], 'a')]));
      expect(onFiles).not.toHaveBeenCalled();
      dispose();
    });
  });
  it('Ctrl/⌘+Enter submits', () => {
    expect(isSubmitChord(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true }))).toBe(true);
    expect(isSubmitChord(new KeyboardEvent('keydown', { key: 'Enter', metaKey: true }))).toBe(true);
    expect(isSubmitChord(new KeyboardEvent('keydown', { key: 'Enter' }))).toBe(false);
  });
});
