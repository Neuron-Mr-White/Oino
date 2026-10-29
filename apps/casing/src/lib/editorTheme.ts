/**
 * CodeMirror 6 dark theme for the Files editor (UNI-155): reuses our
 * token palette (style.css's `--color-*` custom properties) rather than
 * a separate hardcoded theme, so it tracks the app's look.
 */
import { EditorView } from '@codemirror/view';

export const editorTheme = EditorView.theme(
  {
    '&': {
      color: 'var(--color-text)',
      backgroundColor: 'var(--color-bg)',
      height: '100%',
      fontSize: '14px',
    },
    '.cm-content': {
      fontFamily: 'var(--font-mono)',
      caretColor: 'var(--color-text)',
      paddingBottom: '40vh',
    },
    '.cm-scroller': { fontFamily: 'var(--font-mono)' },
    '&.cm-focused .cm-cursor': { borderLeftColor: 'var(--color-text)' },
    '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': {
      backgroundColor: 'rgb(240 48 24 / 30%)',
    },
    // Vim (UNI-190): visual-mode selection layer + the block cursor.
    '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground': { backgroundColor: 'rgb(240 120 24 / 45%)' },
    '.cm-fat-cursor': { background: 'rgb(240 120 24 / 75%) !important', outline: 'none !important', color: 'var(--color-bg) !important' },
    '&:not(.cm-focused) .cm-fat-cursor': { background: 'none !important', outline: '1px solid rgb(240 120 24 / 75%) !important' },
    '.cm-activeLine': { backgroundColor: 'rgb(255 255 255 / 4%)' },
    '.cm-activeLineGutter': { backgroundColor: 'rgb(255 255 255 / 4%)' },
    '.cm-gutters': {
      backgroundColor: 'var(--color-surface-1)',
      color: 'var(--color-text-dim)',
      border: 'none',
    },
    '.cm-lineNumbers .cm-gutterElement': { padding: '0 6px 0 10px' },
    '.cm-searchMatch': { backgroundColor: 'rgb(240 168 24 / 30%)' },
    '.cm-searchMatch-selected': { backgroundColor: 'rgb(240 168 24 / 55%)' },
    '.cm-panels': { backgroundColor: 'var(--color-surface-2)', color: 'var(--color-text)' },
    '.cm-panels.cm-panels-top': { borderBottom: '1px solid var(--color-line)' },
    '.cm-textfield': {
      backgroundColor: 'var(--color-surface-3)',
      color: 'var(--color-text)',
      border: 'none',
      borderRadius: '8px',
    },
    '.cm-button': {
      backgroundImage: 'none',
      backgroundColor: 'var(--color-surface-3)',
      color: 'var(--color-text)',
      border: 'none',
      borderRadius: '8px',
    },
  },
  { dark: true },
);
