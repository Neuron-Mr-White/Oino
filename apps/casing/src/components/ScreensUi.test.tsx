/**
 * UNI-218 QA2 SCREENS pass (F37–F56): the detail header contract and the
 * per-screen consistency rules in ui.css that every screen relies on.
 */
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const src = (p: string) => readFileSync(resolve(__dirname, '..', p), 'utf8');
const css = src('ui.css');

describe('detail header (F39): one 56px bar on every detail pane', () => {
  it('ui.css sizes .detail-head (and the board .task-head) to --detail-head-h 56 on the desktop', () => {
    expect(css).toMatch(/--detail-head-h: 56px/);
    expect(css).toMatch(/html\[data-shell=desktop\] :is\(\.detail-head, \.task-head\) \{[^}]*height: var\(--detail-head-h\)/);
  });

  it.each([
    ['components/chat/ChatView.tsx', 'chat-head detail-head'],
    ['screens/Editor.tsx', 'editor-head detail-head'],
    ['screens/Preview.tsx', 'preview-viewer-head detail-head'],
    ['screens/BrowserView.tsx', 'preview-viewer-head browser-head detail-head'],
    ['components/chat/MermaidViewer.tsx', 'mmd-viewer-head detail-head'],
    ['components/chat/ReviewViewer.tsx', 'review-viewer-head detail-head'],
    ['components/media/Previewer.tsx', 'media-page-head detail-head'],
  ])('%s uses it', (file, cls) => {
    expect(src(file)).toContain(`class="${cls}`);
  });

  it('the remote desktop has a floating RustDesk-style toolbar instead (UNI-271)', () => {
    expect(src('screens/DesktopView.tsx')).toContain('class="desktop-bar"');
    expect(src('screens/DesktopView.tsx')).not.toContain('detail-head');
  });
});
